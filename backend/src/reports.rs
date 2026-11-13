//! 答卷、统计和 CSV 导出。
//!
//! 常规读取使用 SeaORM；统计与导出直接读取持久化的 `outcome` 与 `question_states`，
//! 不再按规则重算整表。分页只读取所需列，导出按 `(submitted_at, id)` 键集分批读取并在有界批次内编码。

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
    sync::Arc,
};

use axum::{
    Json,
    body::Body,
    extract::State,
    http::{HeaderValue, header},
    response::Response,
};
use chrono::{DateTime, FixedOffset, NaiveDate, SecondsFormat, Utc};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, FromQueryResult, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Select, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    AppState,
    auth::AuthUser,
    domain::{AnswerState, Choice, Document, Question, QuestionKind},
    entities::{Outcome, response, version},
    error::{AppError, AppResult, ErrorResponse},
    http::{ApiPath, ApiQuery, Page, Pagination},
    repository,
};

// 本系统记录当代提交时间；上海使用 UTC+08:00，不依赖服务器操作系统的本地时区。
const SHANGHAI_OFFSET: FixedOffset =
    FixedOffset::east_opt(8 * 60 * 60).expect("UTC+08:00 是合法时区偏移");
const TIMEZONE: &str = "Asia/Shanghai (+08:00)";
/// 导出每批读取的行数；批次之间用 `(submitted_at, id)` 键集推进。
const EXPORT_BATCH_ROWS: u64 = 500;
/// CSV 固定前缀列；题目列按发布版本追加。
const CSV_HEADER: [&str; 5] = [
    "receipt_id",
    "outcome",
    "version_id",
    "version_number",
    "submitted_at (Asia/Shanghai +08:00)",
];

/// 所有报表一次只读取一个发布版本；省略版本时选择当前发布版本。
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub(crate) struct ReportFilter {
    pub(crate) version_id: Option<Uuid>,
    /// RFC3339 开始时间，包含该时刻；必须携带 Z 或时区偏移。
    #[schema(format = DateTime)]
    pub(crate) from: Option<String>,
    /// RFC3339 结束时间，不包含该时刻；必须晚于开始时间。
    #[schema(format = DateTime)]
    pub(crate) to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ResponseListQuery {
    #[serde(flatten)]
    filter: ReportFilter,
    #[serde(flatten)]
    pagination: Pagination,
}

/// 文本答案分页：版本、时间范围与分页共用报表筛选口径。
#[derive(Debug, Deserialize)]
struct QuestionResponsesQuery {
    #[serde(flatten)]
    filter: ReportFilter,
    #[serde(flatten)]
    pagination: Pagination,
}

/// 发布版本及本次报表实际使用的时间范围。
#[derive(Serialize, ToSchema)]
pub(crate) struct ReportSelection {
    pub(crate) version_id: Uuid,
    pub(crate) version_number: i32,
    /// 所有返回时间与自然日趋势的展示时区。
    pub(crate) timezone: &'static str,
    pub(crate) from: Option<DateTime<FixedOffset>>,
    pub(crate) to: Option<DateTime<FixedOffset>>,
}

/// 列表不返回内部填写者账号、幂等作用域或哈希。
#[derive(Serialize, ToSchema)]
struct ResponseSummary {
    receipt_id: Uuid,
    submitted_at: DateTime<FixedOffset>,
    /// `complete` 为正常完成；`screen_out` 为筛选结束，同样计入总数但不计入各题分母。
    outcome: Outcome,
}

#[derive(Serialize, ToSchema)]
struct ResponsePage {
    #[serde(flatten)]
    selection: ReportSelection,
    #[serde(flatten)]
    page: Page<ResponseSummary>,
}

/// 题面是提交所绑定的不可变发布版本，不是最新草稿或最新版本。
#[derive(Serialize, ToSchema)]
struct ResponseDetail {
    receipt_id: Uuid,
    version_id: Uuid,
    version_number: i32,
    submitted_at: DateTime<FixedOffset>,
    timezone: &'static str,
    #[schema(value_type = Document)]
    document: Arc<Document>,
    /// 题目 UUID 到规范化答案；未回答的可选题没有对应键。
    answers: BTreeMap<Uuid, Value>,
    /// 每题区分已答、适用未答和规则跳过；提交时持久化，读取时不重算。
    question_states: BTreeMap<Uuid, AnswerState>,
    /// 该答卷的流程终点。
    outcome: Outcome,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct DailyCount {
    /// Asia/Shanghai 自然日；没有提交的日期省略，等价于该日数量为 0。
    pub(crate) date: NaiveDate,
    pub(crate) count: u64,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct OptionFrequency {
    pub(crate) option_id: Uuid,
    pub(crate) label: String,
    pub(crate) count: u64,
    /// 频次 / 本题 `answered_count`，取值 0..1；分母为 0 时为 null。
    pub(crate) proportion: Option<f64>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct RatingFrequency {
    pub(crate) value: i32,
    pub(crate) count: u64,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct TextResponse {
    pub(crate) receipt_id: Uuid,
    pub(crate) submitted_at: DateTime<FixedOffset>,
    /// 原始文本，不自动判断情感或回答有效性。
    pub(crate) value: String,
}

/// 六种题型按实际类型返回对应统计，不使用无意义的通用空字段。
///
/// 文本答案正文不随统计返回，改用按题目分页的文本答案接口读取。
#[derive(Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum QuestionStatisticsData {
    SingleChoice {
        options: Vec<OptionFrequency>,
    },
    MultipleChoice {
        options: Vec<OptionFrequency>,
        /// 多选占比分母仍是回答本题的答卷数，合计可以超过 100%。
        proportion_note: &'static str,
    },
    Text {},
    Textarea {},
    Number {
        /// 排除缺失值，合法 0 参与；无有效回答时为 null。
        mean: Option<f64>,
        min: Option<f64>,
        max: Option<f64>,
    },
    Rating {
        min: i32,
        max: i32,
        /// 按分值升序排列；只返回实际出现过的分值，避免跨度过大的评分范围构造稀疏零桶。
        distribution: Vec<RatingFrequency>,
        /// 排除缺失值，合法 0 参与；无有效回答时为 null。
        mean: Option<f64>,
    },
}

#[derive(Serialize, ToSchema)]
pub(crate) struct QuestionStatistics {
    pub(crate) question_id: Uuid,
    pub(crate) title: String,
    /// 真正回答本题的完成答卷数，不包括缺失值。
    pub(crate) answered_count: u64,
    /// 进入有效路径且可见的完成答卷数，等于已答加适用未答。
    pub(crate) applicable_count: u64,
    /// 适用但没有回答的完成答卷数。
    pub(crate) unanswered_count: u64,
    /// 被分支或显示规则跳过的完成答卷数。
    pub(crate) skipped_count: u64,
    #[serde(flatten)]
    pub(crate) statistics: QuestionStatisticsData,
}

#[derive(Serialize, ToSchema)]
struct StatisticsResponse {
    #[serde(flatten)]
    selection: ReportSelection,
    /// 本次筛选下的全部正式提交数，等于完成数加筛选结束数。
    total_count: u64,
    /// `outcome = complete` 的样本数；各题统计与分母只使用这些样本。
    complete_count: u64,
    /// `outcome = screen_out` 的样本数；计入总数与趋势，不计入各题统计。
    screen_out_count: u64,
    /// 按上海自然日统计全部正式提交（含筛选结束）。
    trend: Vec<DailyCount>,
    questions: Vec<QuestionStatistics>,
}

pub(crate) fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_responses))
        .routes(routes!(response_detail))
        .routes(routes!(question_responses))
        .routes(routes!(statistics))
        .routes(routes!(export_csv))
}

/// 分页查看本人问卷答卷
///
/// 按提交时间升序、回执 UUID 升序稳定排列；总数与分页分别查询。
/// 每行带 `outcome`（`complete` / `screen_out`）。管理员也只能读取自己拥有的问卷；
/// 省略 `version_id` 且问卷未发布时返回 409。
#[utoipa::path(
    get, path = "/api/surveys/{id}/responses", tag = "答卷与报表",
    params(
        ("id" = Uuid, Path, description = "本人问卷 UUID"),
        ReportFilter,
        Pagination
    ),
    responses(
        (status = 200, description = "选定版本的答卷分页，时间为 +08:00", body = ResponsePage),
        (status = 400, description = "路径或查询参数格式错误", body = ErrorResponse),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 404, description = "问卷或版本不可访问", body = ErrorResponse),
        (status = 409, description = "问卷尚无发布版本，code=not_published", body = ErrorResponse),
        (status = 422, description = "分页范围或时间区间无效", body = ErrorResponse),
        (status = 500, description = "服务内部错误", body = ErrorResponse)
    ),
    security(("cookieAuth" = []))
)]
async fn list_responses(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<ResponseListQuery>,
) -> AppResult<Json<ResponsePage>> {
    let bounds = query.pagination.bounds()?;
    let range = TimeRange::parse(&query.filter)?;
    let (version_id, version_number) =
        selected_version_meta(&state.db, user.id, id, query.filter.version_id).await?;
    let responses = filtered_responses(id, version_id, &range);
    let total = responses.clone().count(&state.db).await?;
    let rows = responses
        .select_only()
        .columns([
            response::Column::Id,
            response::Column::SubmittedAt,
            response::Column::Outcome,
        ])
        .order_by_asc(response::Column::SubmittedAt)
        .order_by_asc(response::Column::Id)
        .offset(bounds.offset)
        .limit(bounds.limit)
        .into_model::<ResponseHead>()
        .all(&state.db)
        .await?;
    Ok(Json(ResponsePage {
        selection: range.selection(version_id, version_number),
        page: Page::new(
            rows.into_iter()
                .map(|row| ResponseSummary {
                    receipt_id: row.id,
                    submitted_at: row.submitted_at.with_timezone(&SHANGHAI_OFFSET),
                    outcome: row.outcome,
                })
                .collect(),
            total,
            bounds,
        ),
    }))
}

/// 查看本人问卷的一份答卷及原始版本题面
///
/// 不受当前发布版本、关闭或归档状态影响；不暴露填写者账号和幂等秘密。
/// 三态读取提交时持久化的结果，因此历史答卷与当时的规则判定一致。
#[utoipa::path(
    get, path = "/api/surveys/{id}/responses/{receipt_id}", tag = "答卷与报表",
    params(
        ("id" = Uuid, Path, description = "本人问卷 UUID"),
        ("receipt_id" = Uuid, Path, description = "答卷回执 UUID")
    ),
    responses(
        (status = 200, description = "原版本题面与完整答案，提交时间为 +08:00", body = ResponseDetail),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 404, description = "问卷或答卷不可访问", body = ErrorResponse),
        (status = 400, description = "路径参数格式错误", body = ErrorResponse),
        (status = 500, description = "服务内部错误", body = ErrorResponse)
    ),
    security(("cookieAuth" = []))
)]
async fn response_detail(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath((id, receipt_id)): ApiPath<(Uuid, Uuid)>,
) -> AppResult<Json<ResponseDetail>> {
    repository::owned_survey_meta(&state.db, user.id, id).await?;
    // 显式投影需要的字段，连查询结果也不携带内部身份或幂等哈希。
    let row = response::Entity::find_by_id(receipt_id)
        .filter(response::Column::SurveyId.eq(id))
        .select_only()
        .columns([
            response::Column::Id,
            response::Column::VersionId,
            response::Column::SubmittedAt,
            response::Column::Answers,
            response::Column::QuestionStates,
            response::Column::Outcome,
        ])
        .into_model::<ResponseContent>()
        .one(&state.db)
        .await?
        .ok_or_else(AppError::not_found)?;
    // 版本不可变；按答卷绑定的版本加载题面，答案与三态直接使用提交时的持久化结果。
    let version = repository::load_version(&state.db, &state.versions, id, row.version_id)
        .await?
        .ok_or_else(AppError::not_found)?;
    Ok(Json(ResponseDetail {
        receipt_id: row.id,
        version_id: version.id,
        version_number: version.number,
        submitted_at: row.submitted_at.with_timezone(&SHANGHAI_OFFSET),
        timezone: TIMEZONE,
        document: version.prepared.document_arc(),
        answers: serde_json::from_value(row.answers)?,
        question_states: serde_json::from_value(row.question_states)?,
        outcome: row.outcome,
    }))
}

/// 分页读取某道文本题的答案正文
///
/// 只返回 `outcome = complete` 的答卷；`version_id` 省略时使用当前发布版本。
/// 题目必须属于该版本且为 `text` / `textarea`，否则返回 404 或 422。
/// 文本过滤与分页都在 PostgreSQL 完成，不会把整表答卷读入内存。
#[utoipa::path(
    get, path = "/api/surveys/{id}/questions/{question_id}/responses", tag = "答卷与报表",
    params(
        ("id" = Uuid, Path, description = "本人问卷 UUID"),
        ("question_id" = Uuid, Path, description = "文本题目 UUID"),
        ReportFilter,
        Pagination
    ),
    responses(
        (status = 200, description = "按提交时间升序的文本答案分页", body = Page<TextResponse>),
        (status = 400, description = "路径或查询参数格式错误", body = ErrorResponse),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 404, description = "问卷、版本或题目不可访问", body = ErrorResponse),
        (status = 409, description = "问卷尚无发布版本，code=not_published", body = ErrorResponse),
        (status = 422, description = "题目不是文本题、分页或时间区间无效", body = ErrorResponse),
        (status = 500, description = "服务内部错误", body = ErrorResponse)
    ),
    security(("cookieAuth" = []))
)]
async fn question_responses(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath((id, question_id)): ApiPath<(Uuid, Uuid)>,
    ApiQuery(query): ApiQuery<QuestionResponsesQuery>,
) -> AppResult<Json<Page<TextResponse>>> {
    let bounds = query.pagination.bounds()?;
    let range = TimeRange::parse(&query.filter)?;
    let survey = repository::owned_survey_meta(&state.db, user.id, id).await?;
    let version_id = resolve_version_id(survey.current_version_id, query.filter.version_id)?;
    let version = repository::load_version(&state.db, &state.versions, id, version_id)
        .await?
        .ok_or_else(AppError::not_found)?;
    let is_text = version
        .prepared
        .document()
        .questions
        .iter()
        .find(|question| question.id == question_id)
        .map(|question| {
            matches!(
                &question.kind,
                QuestionKind::Text { .. } | QuestionKind::Textarea { .. }
            )
        })
        .ok_or_else(AppError::not_found)?;
    if !is_text {
        return Err(AppError::validation(
            "该题目不是文本题，不能按文本答案分页读取",
        ));
    }
    // JSONB 存在性过滤与分页都交给 PostgreSQL；只投影当前页需要的三列。
    let question_key = question_id.to_string();
    let responses = filtered_responses(id, version_id, &range)
        .filter(response::Column::Outcome.eq(Outcome::Complete))
        .filter(Expr::cust_with_values(
            "\"responses\".\"answers\" ? $1",
            [question_key.clone()],
        ));
    let total = responses.clone().count(&state.db).await?;
    let rows = responses
        .select_only()
        .columns([
            response::Column::Id,
            response::Column::SubmittedAt,
            response::Column::Answers,
        ])
        .order_by_asc(response::Column::SubmittedAt)
        .order_by_asc(response::Column::Id)
        .offset(bounds.offset)
        .limit(bounds.limit)
        .into_model::<TextRow>()
        .all(&state.db)
        .await?;
    let items = rows
        .into_iter()
        .map(|row| -> AppResult<TextResponse> {
            let Value::Object(mut answers) = row.answers else {
                return Err(invalid_stored_answer());
            };
            let Some(Value::String(value)) = answers.remove(&question_key) else {
                return Err(invalid_stored_answer());
            };
            Ok(TextResponse {
                receipt_id: row.id,
                submitted_at: row.submitted_at.with_timezone(&SHANGHAI_OFFSET),
                value,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(Json(Page::new(items, total, bounds)))
}

/// 统计本人问卷选定版本
///
/// 总数、完成数、筛选结束数、趋势与各题分布都由一次固定结果计算，不会混合版本。
/// `total_count = complete_count + screen_out_count`；趋势按全部正式提交的上海自然日分组。
/// 各题分布、均值与三态计数只统计 `outcome = complete` 的样本，页面必须清晰标明该口径。
/// 选择题占比以该题有效回答数为分母，多选占比之和可超过 100%。
/// 缺失不参与数值统计，合法 0 参与；零分母的占比和均值返回 null。
/// 适用数等于已答加适用未答；适用数加规则跳过数等于完成样本数。
#[utoipa::path(
    get, path = "/api/surveys/{id}/statistics", tag = "答卷与报表",
    params(("id" = Uuid, Path, description = "本人问卷 UUID"), ReportFilter),
    responses(
        (status = 200, description = "完成样本统计；趋势含筛选结束", body = StatisticsResponse),
        (status = 400, description = "路径或查询参数格式错误", body = ErrorResponse),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 404, description = "问卷或版本不可访问", body = ErrorResponse),
        (status = 409, description = "问卷尚无发布版本，code=not_published", body = ErrorResponse),
        (status = 422, description = "时间格式或时间区间无效", body = ErrorResponse),
        (status = 500, description = "服务内部错误", body = ErrorResponse)
    ),
    security(("cookieAuth" = []))
)]
async fn statistics(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(filter): ApiQuery<ReportFilter>,
) -> AppResult<Json<StatisticsResponse>> {
    let range = TimeRange::parse(&filter)?;
    let loaded = load_report(&state, user.id, id, filter.version_id, &range).await?;
    let LoadedReport {
        version_id,
        version_number,
        document,
        stored,
    } = loaded;
    // JSONB 解码与逐题汇总都在同一个阻塞任务内完成，避免两次线程切换。
    let summary = tokio::task::spawn_blocking(move || {
        let rows = decode_rows(stored)?;
        summarize_report(&document, &rows)
    })
    .await
    .map_err(|error| AppError::internal(error))??;
    Ok(Json(StatisticsResponse {
        selection: range.selection(version_id, version_number),
        total_count: summary.total_count,
        complete_count: summary.complete_count,
        screen_out_count: summary.screen_out_count,
        trend: summary.trend,
        questions: summary.questions,
    }))
}

/// 导出本人问卷选定版本的 UTF-8 CSV
///
/// 每次下载重新认证并校验归属；按 `(submitted_at, id)` 键集每 500 行批量读取，逐批编码后按序拼装响应体。
/// 表头固定为回执、流程终点、版本、提交时间（Asia/Shanghai +08:00），随后是题目 UUID 与标题；
/// 带规则的版本在每题后增加状态列。多选是一个字段内的 JSON 数组，缺失留空。
/// 所有字段统一防护公式注入，再按 CSV 规则转义；导出不把整表答卷读入内存。
#[utoipa::path(
    get, path = "/api/surveys/{id}/export.csv", tag = "答卷与报表",
    params(("id" = Uuid, Path, description = "本人问卷 UUID"), ReportFilter),
    responses(
        (status = 200, description = "UTF-8 CSV；公式风险单元格以单引号作为文本保护", body = String,
            content_type = "text/csv; charset=utf-8",
            headers(("Content-Disposition" = String, description = "attachment，文件名只使用问卷和版本 UUID"))),
        (status = 400, description = "路径或查询参数格式错误", body = ErrorResponse),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 404, description = "问卷或版本不可访问", body = ErrorResponse),
        (status = 409, description = "问卷尚无发布版本，code=not_published", body = ErrorResponse),
        (status = 422, description = "时间格式或时间区间无效", body = ErrorResponse),
        (status = 500, description = "服务内部错误", body = ErrorResponse)
    ),
    security(("cookieAuth" = []))
)]
async fn export_csv(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(filter): ApiQuery<ReportFilter>,
) -> AppResult<Response> {
    let range = TimeRange::parse(&filter)?;
    let survey = repository::owned_survey_meta(&state.db, user.id, id).await?;
    let version_id = resolve_version_id(survey.current_version_id, filter.version_id)?;
    let version = repository::load_version(&state.db, &state.versions, id, version_id)
        .await?
        .ok_or_else(AppError::not_found)?;
    let document = version.prepared.document_arc();
    let version_number = version.number;
    // 表头只编码一次，随后按 `(submitted_at, id)` 键集每 500 行读取一批；
    // 批次有界，既不会把整表答案一次性读入，也不会重复解析版本题面。
    let mut bytes = encode_csv_header(&document)?;
    let mut cursor: Option<(DateTime<Utc>, Uuid)> = None;
    loop {
        let stored =
            fetch_export_batch(&state.db, id, version_id, &range, cursor, EXPORT_BATCH_ROWS)
                .await?;
        let Some(last) = stored.last() else { break };
        cursor = Some((last.submitted_at, last.id));
        let batch_is_partial = (stored.len() as u64) < EXPORT_BATCH_ROWS;
        let document = Arc::clone(&document);
        let batch = tokio::task::spawn_blocking(move || {
            let rows = decode_rows(stored)?;
            encode_csv_rows(&document, version_id, version_number, &rows)
        })
        .await
        .map_err(|error| AppError::internal(error))??;
        bytes.extend_from_slice(&batch);
        if batch_is_partial {
            break;
        }
    }
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"survey-{id}-{version_id}.csv\""
        ))
        .map_err(|error| AppError::internal(error))?,
    );
    // no-store / nosniff 由 security::guard 统一添加，这里不重复维护。
    Ok(response)
}

pub(crate) struct TimeRange {
    pub(crate) from: Option<DateTime<Utc>>,
    pub(crate) to: Option<DateTime<Utc>>,
}

impl TimeRange {
    pub(crate) fn parse(filter: &ReportFilter) -> AppResult<Self> {
        fn timestamp(value: &str) -> AppResult<DateTime<Utc>> {
            DateTime::parse_from_rfc3339(value)
                .map(|time| time.with_timezone(&Utc))
                .map_err(|_| AppError::validation("时间必须是包含时区的 RFC3339 格式"))
        }
        let from = filter.from.as_deref().map(timestamp).transpose()?;
        let to = filter.to.as_deref().map(timestamp).transpose()?;
        if let (Some(from), Some(to)) = (from, to)
            && from >= to
        {
            return Err(AppError::validation("开始时间必须早于结束时间"));
        }
        Ok(Self { from, to })
    }

    pub(crate) fn selection(&self, version_id: Uuid, version_number: i32) -> ReportSelection {
        ReportSelection {
            version_id,
            version_number,
            timezone: TIMEZONE,
            from: self.from.map(|time| time.with_timezone(&SHANGHAI_OFFSET)),
            to: self.to.map(|time| time.with_timezone(&SHANGHAI_OFFSET)),
        }
    }
}

/// 显式版本优先；省略时回退到当前发布版本。未发布问卷没有可读取的答卷版本。
pub(crate) fn resolve_version_id(
    current_version_id: Option<Uuid>,
    version_id: Option<Uuid>,
) -> AppResult<Uuid> {
    version_id
        .or(current_version_id)
        .ok_or_else(|| AppError::conflict("not_published", "问卷尚未发布，没有可查看的答卷版本"))
}

/// 答卷列表只需要版本 ID 与编号，不加载版本题面 JSONB。
async fn selected_version_meta(
    db: &impl ConnectionTrait,
    owner_id: Uuid,
    survey_id: Uuid,
    version_id: Option<Uuid>,
) -> AppResult<(Uuid, i32)> {
    let survey = repository::owned_survey_meta(db, owner_id, survey_id).await?;
    let version_id = resolve_version_id(survey.current_version_id, version_id)?;
    version::Entity::find_by_id(version_id)
        .filter(version::Column::SurveyId.eq(survey_id))
        .select_only()
        .columns([version::Column::Id, version::Column::Number])
        .into_tuple::<(Uuid, i32)>()
        .one(db)
        .await?
        .ok_or_else(AppError::not_found)
}

/// 统计共用的单次读取结果：版本标识、不可变题面和原始答卷行。
struct LoadedReport {
    version_id: Uuid,
    version_number: i32,
    document: Arc<Document>,
    stored: Vec<StoredResponseRow>,
}

async fn load_report(
    state: &AppState,
    owner_id: Uuid,
    survey_id: Uuid,
    version_id: Option<Uuid>,
    range: &TimeRange,
) -> AppResult<LoadedReport> {
    let survey = repository::owned_survey_meta(&state.db, owner_id, survey_id).await?;
    let version_id = resolve_version_id(survey.current_version_id, version_id)?;
    // 校验版本归属并编译题面；答卷随后按同一版本读取，解码与汇总交给阻塞线程。
    let version = repository::load_version(&state.db, &state.versions, survey_id, version_id)
        .await?
        .ok_or_else(AppError::not_found)?;
    let stored = filtered_responses(survey_id, version_id, range)
        .select_only()
        .columns([
            response::Column::Id,
            response::Column::SubmittedAt,
            response::Column::Answers,
            response::Column::QuestionStates,
            response::Column::Outcome,
        ])
        .order_by_asc(response::Column::SubmittedAt)
        .order_by_asc(response::Column::Id)
        .into_model::<StoredResponseRow>()
        .all(&state.db)
        .await?;
    Ok(LoadedReport {
        version_id,
        version_number: version.number,
        document: version.prepared.document_arc(),
        stored,
    })
}

#[derive(FromQueryResult)]
struct ResponseHead {
    id: Uuid,
    submitted_at: DateTime<Utc>,
    outcome: Outcome,
}

#[derive(FromQueryResult)]
struct ResponseContent {
    id: Uuid,
    version_id: Uuid,
    submitted_at: DateTime<Utc>,
    answers: Value,
    question_states: Value,
    outcome: Outcome,
}

#[derive(FromQueryResult)]
struct TextRow {
    id: Uuid,
    submitted_at: DateTime<Utc>,
    answers: Value,
}

/// 导出与统计共用的持久化行投影；不携带填写者身份或幂等哈希。
#[derive(FromQueryResult)]
struct StoredResponseRow {
    id: Uuid,
    submitted_at: DateTime<Utc>,
    answers: Value,
    question_states: Value,
    outcome: Outcome,
}

fn filtered_responses(
    survey_id: Uuid,
    version_id: Uuid,
    range: &TimeRange,
) -> Select<response::Entity> {
    let mut query = response::Entity::find()
        .filter(response::Column::SurveyId.eq(survey_id))
        .filter(response::Column::VersionId.eq(version_id));
    if let Some(from) = range.from {
        query = query.filter(response::Column::SubmittedAt.gte(from));
    }
    if let Some(to) = range.to {
        query = query.filter(response::Column::SubmittedAt.lt(to));
    }
    query
}

/// 键集分页读取导出行：时间与回执同时比较，避免同一微秒内的记录被跳过或重复。
async fn fetch_export_batch(
    db: &impl ConnectionTrait,
    survey_id: Uuid,
    version_id: Uuid,
    range: &TimeRange,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: u64,
) -> AppResult<Vec<StoredResponseRow>> {
    let mut query = filtered_responses(survey_id, version_id, range)
        .select_only()
        .columns([
            response::Column::Id,
            response::Column::SubmittedAt,
            response::Column::Answers,
            response::Column::QuestionStates,
            response::Column::Outcome,
        ]);
    if let Some((submitted_at, id)) = after {
        query = query.filter(
            Condition::any()
                .add(response::Column::SubmittedAt.gt(submitted_at))
                .add(
                    Condition::all()
                        .add(response::Column::SubmittedAt.eq(submitted_at))
                        .add(response::Column::Id.gt(id)),
                ),
        );
    }
    Ok(query
        .order_by_asc(response::Column::SubmittedAt)
        .order_by_asc(response::Column::Id)
        .limit(limit)
        .into_model::<StoredResponseRow>()
        .all(db)
        .await?)
}

/// 已解码的一条答卷：答案、持久化三态与流程终点。
pub(crate) struct SnapshotRow {
    pub(crate) receipt_id: Uuid,
    pub(crate) submitted_at: DateTime<Utc>,
    pub(crate) answers: BTreeMap<Uuid, Value>,
    pub(crate) question_states: BTreeMap<Uuid, AnswerState>,
    pub(crate) outcome: Outcome,
}

fn snapshot_row(stored: StoredResponseRow) -> AppResult<SnapshotRow> {
    Ok(SnapshotRow {
        receipt_id: stored.id,
        submitted_at: stored.submitted_at,
        answers: serde_json::from_value(stored.answers)?,
        question_states: serde_json::from_value(stored.question_states)?,
        outcome: stored.outcome,
    })
}

fn decode_rows(stored: Vec<StoredResponseRow>) -> AppResult<Vec<SnapshotRow>> {
    stored.into_iter().map(snapshot_row).collect()
}

/// 统计响应的固定部分：总数、按上海自然日的趋势与逐题统计。
pub(crate) struct ReportStatistics {
    pub(crate) total_count: u64,
    pub(crate) complete_count: u64,
    pub(crate) screen_out_count: u64,
    pub(crate) trend: Vec<DailyCount>,
    pub(crate) questions: Vec<QuestionStatistics>,
}

pub(crate) fn summarize_report(
    document: &Document,
    rows: &[SnapshotRow],
) -> AppResult<ReportStatistics> {
    let mut dates = BTreeMap::new();
    let mut complete_count = 0u64;
    let mut screen_out_count = 0u64;
    for row in rows {
        // 趋势按全部正式提交（含筛选结束）统计，与 total_count 一致。
        *dates
            .entry(
                row.submitted_at
                    .with_timezone(&SHANGHAI_OFFSET)
                    .date_naive(),
            )
            .or_insert(0) += 1;
        match row.outcome {
            Outcome::Complete => complete_count += 1,
            Outcome::ScreenOut => screen_out_count += 1,
        }
    }
    let questions = document
        .questions
        .iter()
        .map(|question| summarize_question(question, rows))
        .collect::<AppResult<Vec<_>>>()?;
    Ok(ReportStatistics {
        total_count: complete_count + screen_out_count,
        complete_count,
        screen_out_count,
        trend: dates
            .into_iter()
            .map(|(date, count)| DailyCount { date, count })
            .collect(),
        questions,
    })
}

/// 各题只统计 `outcome = complete` 的样本；筛选结束不计入任何题目分母。
pub(crate) fn summarize_question(
    question: &Question,
    rows: &[SnapshotRow],
) -> AppResult<QuestionStatistics> {
    let complete: Vec<&SnapshotRow> = rows
        .iter()
        .filter(|row| row.outcome == Outcome::Complete)
        .collect();
    summarize_complete_question(question, &complete)
}

fn summarize_complete_question(
    question: &Question,
    rows: &[&SnapshotRow],
) -> AppResult<QuestionStatistics> {
    let mut answered_count = 0u64;
    let mut skipped_count = 0u64;
    let is_multiple = matches!(&question.kind, QuestionKind::MultipleChoice { .. });
    let is_textarea = matches!(&question.kind, QuestionKind::Textarea { .. });
    let statistics = match &question.kind {
        QuestionKind::SingleChoice { options } | QuestionKind::MultipleChoice { options, .. } => {
            let options = summarize_choices(
                options,
                question.id,
                rows,
                &mut answered_count,
                &mut skipped_count,
            )?;
            if is_multiple {
                QuestionStatisticsData::MultipleChoice {
                    options,
                    proportion_note: "各选项频次除以本题有效回答数，占比合计可以超过 100%",
                }
            } else {
                QuestionStatisticsData::SingleChoice { options }
            }
        }
        QuestionKind::Text { .. } | QuestionKind::Textarea { .. } => {
            // 正文通过文本答案分页接口读取；统计只保留计数与三态。
            for row in rows {
                let _ = observe_row(row, question.id, &mut answered_count, &mut skipped_count);
            }
            if is_textarea {
                QuestionStatisticsData::Textarea {}
            } else {
                QuestionStatisticsData::Text {}
            }
        }
        QuestionKind::Number { .. } => {
            let mut mean = 0.0;
            let mut min: Option<f64> = None;
            let mut max: Option<f64> = None;
            for row in rows {
                if let Some(value) =
                    observe_row(row, question.id, &mut answered_count, &mut skipped_count)
                {
                    let value = value.as_f64().ok_or_else(invalid_stored_answer)?;
                    update_mean(&mut mean, value, answered_count);
                    min = Some(min.map_or(value, |previous| previous.min(value)));
                    max = Some(max.map_or(value, |previous| previous.max(value)));
                }
            }
            QuestionStatisticsData::Number {
                mean: (answered_count > 0).then_some(mean),
                min,
                max,
            }
        }
        QuestionKind::Rating { min, max, .. } => {
            let mut distribution = BTreeMap::<i32, u64>::new();
            let mut sum = 0.0;
            for row in rows {
                if let Some(value) =
                    observe_row(row, question.id, &mut answered_count, &mut skipped_count)
                {
                    // 写入前已校验整数与 i32 范围；规范化 JSON 可能保存为 0.0。
                    let value = value.as_f64().ok_or_else(invalid_stored_answer)?;
                    *distribution.entry(value as i32).or_default() += 1;
                    sum += value;
                }
            }
            QuestionStatisticsData::Rating {
                min: *min,
                max: *max,
                distribution: distribution
                    .into_iter()
                    .map(|(value, count)| RatingFrequency { value, count })
                    .collect(),
                mean: (answered_count > 0).then(|| sum / answered_count as f64),
            }
        }
    };
    let applicable_count = rows.len() as u64 - skipped_count;
    Ok(QuestionStatistics {
        question_id: question.id,
        title: question.title.clone(),
        answered_count,
        applicable_count,
        unanswered_count: applicable_count - answered_count,
        skipped_count,
        statistics,
    })
}

// 统计单行答题状态：规则跳过单独计数，命中答案则累加有效回答数并返回答案原文。
fn observe_row<'a>(
    row: &'a SnapshotRow,
    question_id: Uuid,
    answered_count: &mut u64,
    skipped_count: &mut u64,
) -> Option<&'a Value> {
    if row.question_states.get(&question_id) == Some(&AnswerState::Skipped) {
        *skipped_count += 1;
    }
    let value = row.answers.get(&question_id)?;
    *answered_count += 1;
    Some(value)
}

fn summarize_choices(
    options: &[Choice],
    question_id: Uuid,
    rows: &[&SnapshotRow],
    answered_count: &mut u64,
    skipped_count: &mut u64,
) -> AppResult<Vec<OptionFrequency>> {
    let mut counts = HashMap::<&str, u64>::with_capacity(options.len());
    for row in rows {
        let Some(value) = observe_row(row, question_id, answered_count, skipped_count) else {
            continue;
        };
        match value {
            Value::String(option) => *counts.entry(option.as_str()).or_default() += 1,
            Value::Array(options) => {
                for option in options {
                    *counts
                        .entry(option.as_str().ok_or_else(invalid_stored_answer)?)
                        .or_default() += 1;
                }
            }
            _ => return Err(invalid_stored_answer()),
        }
    }
    let total_answered = *answered_count;
    let mut id_buf = Uuid::encode_buffer();
    Ok(options
        .iter()
        .map(|option| {
            let id_str: &str = option.id.hyphenated().encode_lower(&mut id_buf);
            let count = counts.get(id_str).copied().unwrap_or(0);
            OptionFrequency {
                option_id: option.id,
                label: option.label.clone(),
                count,
                proportion: (total_answered > 0).then(|| count as f64 / total_answered as f64),
            }
        })
        .collect())
}

pub(crate) fn invalid_stored_answer() -> AppError {
    // 不在诊断信息里拼接答案正文。
    AppError::internal("已保存答案与发布题型不一致")
}

/// 运行均值：先按已有计数还原总和，再并入新值；`count` 从 1 开始。
///
/// `(mean * (count - 1) + value) / count` 对最大正负浮点也保持有限结果，
/// 且只有一次乘加，不引入额外的溢出防护分支。
pub(crate) fn update_mean(mean: &mut f64, value: f64, count: u64) {
    *mean = (*mean * (count.saturating_sub(1) as f64) + value) / count as f64;
}

fn outcome_text(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Complete => "complete",
        Outcome::ScreenOut => "screen_out",
    }
}

fn state_text(state: AnswerState) -> &'static str {
    match state {
        AnswerState::Answered => "已答",
        AnswerState::Unanswered => "适用未答",
        AnswerState::Skipped => "规则跳过",
    }
}

fn csv_writer() -> csv::Writer<Vec<u8>> {
    csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_writer(Vec::with_capacity(4 << 10))
}

#[track_caller]
fn write_cell(writer: &mut csv::Writer<Vec<u8>>, cell: &str) -> AppResult<()> {
    writer
        .write_field(formula_safe(cell).as_bytes())
        .map_err(|error| AppError::internal(error))
}

fn finish_writer(writer: csv::Writer<Vec<u8>>) -> AppResult<Vec<u8>> {
    writer
        .into_inner()
        .map_err(|error| AppError::internal(error))
}

/// CSV 表头：固定前缀列 + 题目列（有规则时每题后追加状态列）。
pub(crate) fn encode_csv_header(document: &Document) -> AppResult<Vec<u8>> {
    let mut writer = csv_writer();
    for header in CSV_HEADER {
        write_cell(&mut writer, header)?;
    }
    let with_states = document.logic.is_some();
    // 单个缓冲区复用整次表头编码，避免每个单元格都分配新字符串。
    let mut header_buf: String = String::with_capacity(128);
    for question in &document.questions {
        header_buf.clear();
        write!(&mut header_buf, "{} [{}]", question.title, question.id)
            .map_err(|error| AppError::internal(error))?;
        write_cell(&mut writer, &header_buf)?;
        if with_states {
            header_buf.clear();
            write!(&mut header_buf, "状态 [{}]", question.id)
                .map_err(|error| AppError::internal(error))?;
            write_cell(&mut writer, &header_buf)?;
        }
    }
    writer
        .write_record(std::iter::empty::<&[u8]>())
        .map_err(|error| AppError::internal(error))?;
    finish_writer(writer)
}

/// 一批答卷的 CSV 记录（不含表头）；调用方按批拼接。
pub(crate) fn encode_csv_rows(
    document: &Document,
    version_id: Uuid,
    version_number: i32,
    rows: &[SnapshotRow],
) -> AppResult<Vec<u8>> {
    let mut writer = csv_writer();
    let version_id = version_id.to_string();
    let version_number = version_number.to_string();
    let with_states = document.logic.is_some();
    let mut id_buf = Uuid::encode_buffer();
    let mut json_buf: Vec<u8> = Vec::with_capacity(128);
    for row in rows {
        write_cell(
            &mut writer,
            row.receipt_id.hyphenated().encode_lower(&mut id_buf),
        )?;
        write_cell(&mut writer, outcome_text(row.outcome))?;
        write_cell(&mut writer, &version_id)?;
        write_cell(&mut writer, &version_number)?;
        let timestamp = row
            .submitted_at
            .with_timezone(&SHANGHAI_OFFSET)
            .to_rfc3339_opts(SecondsFormat::AutoSi, false);
        write_cell(&mut writer, &timestamp)?;
        for question in &document.questions {
            match row.answers.get(&question.id) {
                None => write_cell(&mut writer, "")?,
                Some(Value::String(value)) => write_cell(&mut writer, value)?,
                // 多选保持 JSON 数组，不用逗号连接，以免丢失集合与 CSV 字段边界。
                Some(value) => {
                    json_buf.clear();
                    serde_json::to_writer(&mut json_buf, value)?;
                    let json = std::str::from_utf8(&json_buf)
                        .map_err(|error| AppError::internal(error))?;
                    write_cell(&mut writer, json)?;
                }
            }
            if with_states {
                let state = row
                    .question_states
                    .get(&question.id)
                    .copied()
                    .ok_or_else(invalid_stored_answer)?;
                write_cell(&mut writer, state_text(state))?;
            }
        }
        writer
            .write_record(std::iter::empty::<&[u8]>())
            .map_err(|error| AppError::internal(error))?;
    }
    finish_writer(writer)
}

/// 表头与全部行的完整编码；供小批量测试和一次性场景使用。
#[cfg(test)]
pub(crate) fn encode_csv(
    document: &Document,
    version_id: Uuid,
    version_number: i32,
    rows: &[SnapshotRow],
) -> AppResult<Vec<u8>> {
    let mut bytes = encode_csv_header(document)?;
    bytes.extend_from_slice(&encode_csv_rows(
        document,
        version_id,
        version_number,
        rows,
    )?);
    Ok(bytes)
}

pub(crate) fn formula_safe(value: &str) -> Cow<'_, str> {
    // 只检查前缀，不删除用户原文。除了空白和控制字符，还覆盖 BOM 与常见不可见格式符。
    let visible = value.trim_start_matches(|ch: char| {
        ch.is_whitespace()
            || ch.is_control()
            || matches!(ch,
                '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' |
                '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}'
            )
    });
    if visible.starts_with(['=', '+', '-', '@']) {
        Cow::Owned(format!("'{value}"))
    } else {
        Cow::Borrowed(value)
    }
}
