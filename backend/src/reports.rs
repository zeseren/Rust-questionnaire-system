//! 答卷、统计和 CSV 导出。
//!
//! 常规读取使用 SeaORM；统计和导出只读取一次答卷结果，避免同一响应内的数据快照漂移。

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{DateTime, FixedOffset, NaiveDate, SecondsFormat, Utc};
use sea_orm::{
    AccessMode, ConnectionTrait, DatabaseConnection, FromQueryResult, IsolationLevel,
    TransactionTrait,
    entity::{ColumnTrait, EntityTrait},
    query::{PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Select},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    AppState,
    auth::AuthUser,
    domain::{AnswerState, Choice, Document, FlowEvaluator, Question, QuestionKind},
    entities::{response, version},
    error::{AppError, AppResult, ErrorResponse},
    http::{ApiPath, ApiQuery, Pagination},
    repository,
};

// 本系统记录当代提交时间；上海使用 UTC+08:00，不依赖服务器操作系统的本地时区。
const SHANGHAI_OFFSET: FixedOffset =
    FixedOffset::east_opt(8 * 60 * 60).expect("UTC+08:00 是合法时区偏移");
const TIMEZONE: &str = "Asia/Shanghai (+08:00)";

/// 所有报表一次只读取一个发布版本；省略版本时选择当前发布版本。
#[derive(Debug, Deserialize, ToSchema)]
struct ReportFilter {
    version_id: Option<Uuid>,
    /// RFC3339 开始时间，包含该时刻；必须携带 Z 或时区偏移，精度最多 6 位小数（微秒）。
    #[schema(format = DateTime)]
    from: Option<String>,
    /// RFC3339 结束时间，不包含该时刻；必须晚于开始时间，精度最多 6 位小数（微秒）。
    #[schema(format = DateTime)]
    to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ResponseListQuery {
    #[serde(flatten)]
    filter: ReportFilter,
    #[serde(flatten)]
    pagination: Pagination,
}

/// 发布版本及本次报表实际使用的时间范围。
#[derive(Serialize, ToSchema)]
struct ReportSelection {
    version_id: Uuid,
    version_number: i32,
    /// 所有返回时间与自然日趋势的展示时区。
    timezone: &'static str,
    from: Option<DateTime<FixedOffset>>,
    to: Option<DateTime<FixedOffset>>,
}

/// 列表不返回内部填写者账号、幂等作用域或哈希。
#[derive(Serialize, ToSchema)]
struct ResponseSummary {
    receipt_id: Uuid,
    submitted_at: DateTime<FixedOffset>,
}

#[derive(Serialize, ToSchema)]
struct ResponsePage {
    #[serde(flatten)]
    selection: ReportSelection,
    page: u64,
    page_size: u64,
    total: u64,
    items: Vec<ResponseSummary>,
}

/// 题面是提交所绑定的不可变发布版本，不是最新草稿或最新版本。
#[derive(Serialize, ToSchema)]
struct ResponseDetail {
    receipt_id: Uuid,
    version_id: Uuid,
    version_number: i32,
    submitted_at: DateTime<FixedOffset>,
    timezone: &'static str,
    document: Document,
    /// 题目 UUID 到规范化答案；未回答的可选题没有对应键。
    answers: BTreeMap<Uuid, Value>,
    /// 每题区分已答、适用未答和规则跳过；按此答卷的历史版本重算。
    question_states: BTreeMap<Uuid, AnswerState>,
}

#[derive(Serialize, ToSchema)]
struct DailyCount {
    /// Asia/Shanghai 自然日；没有提交的日期省略，等价于该日数量为 0。
    date: NaiveDate,
    count: u64,
}

#[derive(Serialize, ToSchema)]
struct OptionFrequency {
    option_id: Uuid,
    label: String,
    count: u64,
    /// 频次 / 本题 answered_count，取值 0..1；分母为 0 时为 null。
    proportion: Option<f64>,
}

#[derive(Serialize, ToSchema)]
struct RatingFrequency {
    value: i32,
    count: u64,
}

#[derive(Serialize, ToSchema)]
struct TextResponse {
    receipt_id: Uuid,
    submitted_at: DateTime<FixedOffset>,
    /// 原始文本，不自动判断情感或回答有效性。
    value: String,
}

/// 六种题型按实际类型返回对应统计，不使用无意义的通用空字段。
#[derive(Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
enum QuestionStatisticsData {
    SingleChoice {
        options: Vec<OptionFrequency>,
    },
    MultipleChoice {
        options: Vec<OptionFrequency>,
        /// 多选占比分母仍是回答本题的答卷数，合计可以超过 100%。
        proportion_note: &'static str,
    },
    Text {
        answers: Vec<TextResponse>,
    },
    Textarea {
        answers: Vec<TextResponse>,
    },
    Number {
        /// 排除缺失值，合法 0 参与；无有效回答时为 null。
        mean: Option<f64>,
        min: Option<f64>,
        max: Option<f64>,
    },
    Rating {
        min: i32,
        max: i32,
        /// 按分值升序排列；没有出现的分值频次为 0。
        distribution: Vec<RatingFrequency>,
        /// 排除缺失值，合法 0 参与；无有效回答时为 null。
        mean: Option<f64>,
    },
}

#[derive(Serialize, ToSchema)]
struct QuestionStatistics {
    question_id: Uuid,
    title: String,
    /// 真正回答本题的答卷数，不包括缺失值。
    answered_count: u64,
    /// 进入有效路径且可见的答卷数，等于已答加适用未答。
    applicable_count: u64,
    /// 适用但没有回答的答卷数。
    unanswered_count: u64,
    /// 被分支或显示规则跳过的答卷数。
    skipped_count: u64,
    #[serde(flatten)]
    statistics: QuestionStatisticsData,
}

#[derive(Serialize, ToSchema)]
struct StatisticsResponse {
    #[serde(flatten)]
    selection: ReportSelection,
    total_count: u64,
    trend: Vec<DailyCount>,
    questions: Vec<QuestionStatistics>,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/surveys/{id}/responses", get(list_responses))
        .route(
            "/api/surveys/{id}/responses/{receipt_id}",
            get(response_detail),
        )
        .route("/api/surveys/{id}/statistics", get(statistics))
        .route("/api/surveys/{id}/export.csv", get(export_csv))
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(list_responses, response_detail, statistics, export_csv),
    components(schemas(
        ReportFilter,
        ResponseListQuery,
        ReportSelection,
        ResponseSummary,
        ResponsePage,
        ResponseDetail,
        DailyCount,
        OptionFrequency,
        RatingFrequency,
        TextResponse,
        QuestionStatisticsData,
        QuestionStatistics,
        StatisticsResponse,
        ErrorResponse
    ))
)]
pub(crate) struct ApiDoc;

/// 分页查看本人问卷答卷
///
/// 按提交时间升序、回执 UUID 升序稳定排列。一个请求内的总数与分页数据来自同一只读快照。
/// 管理员也只能读取自己拥有的问卷；省略 version_id 且问卷未发布时返回 409。
#[utoipa::path(
    get, path = "/api/surveys/{id}/responses", tag = "答卷与报表",
    params(
        ("id" = Uuid, Path, description = "本人问卷 UUID"),
        ("version_id" = Option<Uuid>, Query, description = "发布版本 UUID，默认当前版本"),
        ("from" = Option<DateTime<Utc>>, Query, description = "RFC3339 开始时刻，包含；最多微秒精度（6位小数），超出返回422"),
        ("to" = Option<DateTime<Utc>>, Query, description = "RFC3339 结束时刻，不包含；最多微秒精度（6位小数），超出返回422"),
        ("page" = Option<u64>, Query, description = "页码，从 1 开始，默认 1"),
        ("page_size" = Option<u64>, Query, description = "每页 1..100 条，默认 20")
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
    let (offset, limit) = query.pagination.bounds()?;
    let range = TimeRange::parse(&query.filter)?;
    let tx = state
        .db
        .begin_with_config(
            Some(IsolationLevel::RepeatableRead),
            Some(AccessMode::ReadOnly),
        )
        .await?;
    let version = selected_version(&tx, user.id, id, query.filter.version_id).await?;
    let responses = filtered_responses(id, version.id, &range);
    let total = responses.clone().count(&tx).await?;
    let rows = responses
        .select_only()
        .columns([response::Column::Id, response::Column::SubmittedAt])
        .order_by_asc(response::Column::SubmittedAt)
        .order_by_asc(response::Column::Id)
        .offset(offset)
        .limit(limit)
        .into_model::<ResponseHead>()
        .all(&tx)
        .await?;
    tx.commit().await?;
    Ok(Json(ResponsePage {
        selection: range.selection(version.id, version.number),
        page: offset / limit + 1,
        page_size: limit,
        total,
        items: rows
            .into_iter()
            .map(|row| ResponseSummary {
                receipt_id: row.id,
                submitted_at: row.submitted_at.with_timezone(&SHANGHAI_OFFSET),
            })
            .collect(),
    }))
}

/// 查看本人问卷的一份答卷及原始版本题面
///
/// 不受当前发布版本、关闭或归档状态影响；不暴露填写者账号和幂等秘密。
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
    repository::owned_survey(&state.db, user.id, id).await?;
    // 显式投影需要的字段，连查询结果也不携带内部身份或幂等哈希。
    let row = response::Entity::find_by_id(receipt_id)
        .filter(response::Column::SurveyId.eq(id))
        .select_only()
        .columns([
            response::Column::Id,
            response::Column::VersionId,
            response::Column::SubmittedAt,
            response::Column::Answers,
        ])
        .into_model::<ResponseContent>()
        .one(&state.db)
        .await?
        .ok_or_else(AppError::not_found)?;
    let version = version::Entity::find_by_id(row.version_id)
        .filter(version::Column::SurveyId.eq(id))
        .one(&state.db)
        .await?
        .ok_or_else(AppError::not_found)?;
    let document: Document = serde_json::from_value(version.document)?;
    let evaluator = report_evaluator(&document)?;
    let row = snapshot_row(
        &document,
        evaluator.as_ref(),
        row.id,
        row.submitted_at,
        serde_json::from_value(row.answers)?,
    )?;
    Ok(Json(ResponseDetail {
        receipt_id: row.receipt_id,
        version_id: version.id,
        version_number: version.number,
        submitted_at: row.submitted_at.with_timezone(&SHANGHAI_OFFSET),
        timezone: TIMEZONE,
        document,
        answers: row.answers,
        question_states: row.question_states,
    }))
}

/// 统计本人问卷选定版本
///
/// 总数、趋势、分布和分母均由一次 SELECT 的固定结果计算；不会混合版本。
/// 选择题占比以该题有效回答数为分母，多选占比之和可超过 100%。
/// 缺失不参与数值统计，合法 0 参与；零分母的占比和均值返回 null。
/// 适用数等于已答加适用未答；适用数加规则跳过数等于本次答卷总数。
#[utoipa::path(
    get, path = "/api/surveys/{id}/statistics", tag = "答卷与报表",
    params(
        ("id" = Uuid, Path, description = "本人问卷 UUID"),
        ("version_id" = Option<Uuid>, Query, description = "发布版本 UUID，默认当前版本"),
        ("from" = Option<DateTime<Utc>>, Query, description = "RFC3339 开始时刻，包含；最多微秒精度（6位小数），超出返回422"),
        ("to" = Option<DateTime<Utc>>, Query, description = "RFC3339 结束时刻，不包含；最多微秒精度（6位小数），超出返回422")
    ),
    responses(
        (status = 200, description = "一致快照统计，趋势按上海自然日分组", body = StatisticsResponse),
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
    let version = selected_version(&state.db, user.id, id, filter.version_id).await?;
    let document: Document = serde_json::from_value(version.document)?;
    let rows = response_snapshot(&state.db, user.id, id, version.id, &range, &document).await?;
    let mut dates = BTreeMap::new();
    for row in &rows {
        *dates
            .entry(
                row.submitted_at
                    .with_timezone(&SHANGHAI_OFFSET)
                    .date_naive(),
            )
            .or_insert(0) += 1;
    }
    let questions = document
        .questions
        .into_iter()
        .map(|question| summarize_question(question, &rows))
        .collect::<AppResult<Vec<_>>>()?;
    Ok(Json(StatisticsResponse {
        selection: range.selection(version.id, version.number),
        total_count: rows.len() as u64,
        trend: dates
            .into_iter()
            .map(|(date, count)| DailyCount { date, count })
            .collect(),
        questions,
    }))
}

/// 导出本人问卷选定版本的 UTF-8 CSV
///
/// 每次下载重新认证并校验归属；一次 SELECT 固定导出行，按提交时间和回执升序。
/// 表头包含版本、提交时间（Asia/Shanghai +08:00）、题目 UUID 和标题。
/// 多选是一个字段内的 JSON 数组，缺失留空；带规则的版本在每题后增加状态列，区分适用未答与规则跳过。
/// 无规则历史版本的列与值格式保持不变；所有字段统一防护公式注入，再按 CSV 规则转义。
/// 为保持电子表格安全，以等号、加减号或 @ 开头的单元格会加单引号，含前导空白/控制字符的绕过同样处理。
#[utoipa::path(
    get, path = "/api/surveys/{id}/export.csv", tag = "答卷与报表",
    params(
        ("id" = Uuid, Path, description = "本人问卷 UUID"),
        ("version_id" = Option<Uuid>, Query, description = "发布版本 UUID，默认当前版本"),
        ("from" = Option<DateTime<Utc>>, Query, description = "RFC3339 开始时刻，包含；最多微秒精度（6位小数），超出返回422"),
        ("to" = Option<DateTime<Utc>>, Query, description = "RFC3339 结束时刻，不包含；最多微秒精度（6位小数），超出返回422")
    ),
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
    let version = selected_version(&state.db, user.id, id, filter.version_id).await?;
    let document: Document = serde_json::from_value(version.document)?;
    let rows = response_snapshot(&state.db, user.id, id, version.id, &range, &document).await?;
    let bytes = encode_csv(&document, version.id, version.number, &rows)?;
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"survey-{id}-{}.csv\"",
            version.id
        ))
        .map_err(|error| AppError::internal(error))?,
    );
    // no-store / nosniff 由 security::guard 统一添加，这里不重复维护。
    Ok(response)
}

struct TimeRange {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

impl TimeRange {
    fn parse(filter: &ReportFilter) -> AppResult<Self> {
        fn timestamp(value: &str) -> AppResult<DateTime<Utc>> {
            let time = DateTime::parse_from_rfc3339(value)
                .map_err(|_| AppError::validation("时间必须是包含时区的 RFC3339 格式"))?;
            // PostgreSQL 只保存微秒；也检查原文，避免 chrono 先截断超过纳秒的非零尾数。
            let sub_microsecond = value.split_once('.').is_some_and(|(_, fraction)| {
                fraction
                    .bytes()
                    .take_while(u8::is_ascii_digit)
                    .skip(6)
                    .any(|digit| digit != b'0')
            });
            if !time.timestamp_subsec_nanos().is_multiple_of(1000) || sub_microsecond {
                return Err(AppError::validation(
                    "时间精度最多为 6 位小数（微秒），不能静默截断",
                ));
            }
            Ok(time.with_timezone(&Utc))
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

    fn selection(&self, version_id: Uuid, version_number: i32) -> ReportSelection {
        ReportSelection {
            version_id,
            version_number,
            timezone: TIMEZONE,
            from: self.from.map(|time| time.with_timezone(&SHANGHAI_OFFSET)),
            to: self.to.map(|time| time.with_timezone(&SHANGHAI_OFFSET)),
        }
    }
}

async fn selected_version(
    db: &impl ConnectionTrait,
    owner_id: Uuid,
    survey_id: Uuid,
    version_id: Option<Uuid>,
) -> AppResult<version::Model> {
    let survey = repository::owned_survey(db, owner_id, survey_id).await?;
    let version_id = version_id
        .or(survey.current_version_id)
        .ok_or_else(|| AppError::conflict("not_published", "问卷尚未发布，没有可查看的答卷版本"))?;
    version::Entity::find_by_id(version_id)
        .filter(version::Column::SurveyId.eq(survey_id))
        .one(db)
        .await?
        .ok_or_else(AppError::not_found)
}

#[derive(FromQueryResult)]
struct ResponseHead {
    id: Uuid,
    submitted_at: DateTime<Utc>,
}

#[derive(FromQueryResult)]
struct ResponseContent {
    id: Uuid,
    version_id: Uuid,
    submitted_at: DateTime<Utc>,
    answers: Value,
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

// 元组对应固定 SELECT 的三列；Json 直接解码规范化答案，避免再复制一份 JSON 树。
type StoredRow = (
    Uuid,
    DateTime<Utc>,
    sqlx::types::Json<BTreeMap<Uuid, Value>>,
);

// 适用性只来自历史版本重建；不按当前草稿推断，也不从答案是否缺失猜测跳题。
struct SnapshotRow {
    receipt_id: Uuid,
    submitted_at: DateTime<Utc>,
    answers: BTreeMap<Uuid, Value>,
    question_states: BTreeMap<Uuid, AnswerState>,
}

fn report_evaluator(document: &Document) -> AppResult<Option<FlowEvaluator<'_>>> {
    document
        .logic
        .as_ref()
        .map(|_| FlowEvaluator::new(document))
        .transpose()
        .map_err(|error| AppError::internal(error))
}

fn snapshot_row(
    document: &Document,
    evaluator: Option<&FlowEvaluator<'_>>,
    receipt_id: Uuid,
    submitted_at: DateTime<Utc>,
    answers: BTreeMap<Uuid, Value>,
) -> AppResult<SnapshotRow> {
    let (answers, question_states) = if let Some(evaluator) = evaluator {
        let result = evaluator
            .evaluate_stored(answers)
            .map_err(|error| AppError::internal(error))?;
        (result.answers, result.question_states)
    } else {
        let states = document
            .questions
            .iter()
            .map(|question| {
                (
                    question.id,
                    if answers.contains_key(&question.id) {
                        AnswerState::Answered
                    } else {
                        AnswerState::Unanswered
                    },
                )
            })
            .collect();
        (answers, states)
    };
    Ok(SnapshotRow {
        receipt_id,
        submitted_at,
        answers,
        question_states,
    })
}

async fn response_snapshot(
    db: &DatabaseConnection,
    owner_id: Uuid,
    survey_id: Uuid,
    version_id: Uuid,
    range: &TimeRange,
    document: &Document,
) -> AppResult<Vec<SnapshotRow>> {
    // 单条 PostgreSQL SELECT 自带一致快照。这里不处于 ORM 事务内，直接复用同一个连接池。
    // SQL 自身仍限制 owner_id，不能因为使用 SQLx 就绕过权限。
    let stored: Vec<StoredRow> = sqlx::query_as(
        "SELECT r.id, r.submitted_at, r.answers
         FROM responses AS r
         INNER JOIN surveys AS s ON s.id = r.survey_id
         WHERE s.owner_id = $1 AND r.survey_id = $2 AND r.version_id = $3
           AND ($4::timestamptz IS NULL OR r.submitted_at >= $4)
           AND ($5::timestamptz IS NULL OR r.submitted_at < $5)
         ORDER BY r.submitted_at ASC, r.id ASC",
    )
    .bind(owner_id)
    .bind(survey_id)
    .bind(version_id)
    .bind(range.from)
    .bind(range.to)
    .fetch_all(db.get_postgres_connection_pool())
    .await?;
    let evaluator = report_evaluator(document)?;
    stored
        .into_iter()
        .map(|(id, submitted_at, answers)| {
            snapshot_row(document, evaluator.as_ref(), id, submitted_at, answers.0)
        })
        .collect()
}

fn summarize_question(question: Question, rows: &[SnapshotRow]) -> AppResult<QuestionStatistics> {
    let answered_count = rows
        .iter()
        .filter(|row| row.answers.contains_key(&question.id))
        .count() as u64;
    let skipped_count = rows
        .iter()
        .filter(|row| row.question_states.get(&question.id) == Some(&AnswerState::Skipped))
        .count() as u64;
    let applicable_count = rows.len() as u64 - skipped_count;
    let is_multiple = matches!(&question.kind, QuestionKind::MultipleChoice { .. });
    let is_textarea = matches!(&question.kind, QuestionKind::Textarea { .. });
    let statistics = match question.kind {
        QuestionKind::SingleChoice { options } | QuestionKind::MultipleChoice { options, .. } => {
            let options = summarize_choices(options, question.id, answered_count, rows)?;
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
            let mut answers = Vec::with_capacity(answered_count as usize);
            for SnapshotRow {
                receipt_id,
                submitted_at,
                answers: values,
                ..
            } in rows
            {
                if let Some(value) = values.get(&question.id) {
                    answers.push(TextResponse {
                        receipt_id: *receipt_id,
                        submitted_at: submitted_at.with_timezone(&SHANGHAI_OFFSET),
                        value: value.as_str().ok_or_else(invalid_stored_answer)?.to_owned(),
                    });
                }
            }
            if is_textarea {
                QuestionStatisticsData::Textarea { answers }
            } else {
                QuestionStatisticsData::Text { answers }
            }
        }
        QuestionKind::Number { .. } => {
            let mut count = 0;
            let mut mean = 0.0;
            let mut min: Option<f64> = None;
            let mut max: Option<f64> = None;
            for row in rows {
                if let Some(value) = row.answers.get(&question.id) {
                    let value = value.as_f64().ok_or_else(invalid_stored_answer)?;
                    count += 1;
                    update_mean(&mut mean, value, count);
                    min = Some(min.map_or(value, |previous| previous.min(value)));
                    max = Some(max.map_or(value, |previous| previous.max(value)));
                }
            }
            QuestionStatisticsData::Number {
                mean: (count > 0).then_some(mean),
                min,
                max,
            }
        }
        QuestionKind::Rating { min, max, .. } => {
            let mut distribution = BTreeMap::<i32, u64>::new();
            let mut sum = 0.0;
            for row in rows {
                if let Some(value) = row.answers.get(&question.id) {
                    // 写入前已校验整数与 i32 范围；规范化 JSON 可能保存为 0.0。
                    let value = value.as_f64().ok_or_else(invalid_stored_answer)?;
                    *distribution.entry(value as i32).or_default() += 1;
                    sum += value;
                }
            }
            QuestionStatisticsData::Rating {
                min,
                max,
                distribution: distribution
                    .into_iter()
                    .map(|(value, count)| RatingFrequency { value, count })
                    .collect(),
                mean: (answered_count > 0).then(|| sum / answered_count as f64),
            }
        }
    };
    Ok(QuestionStatistics {
        question_id: question.id,
        title: question.title,
        answered_count,
        applicable_count,
        unanswered_count: applicable_count - answered_count,
        skipped_count,
        statistics,
    })
}

fn summarize_choices(
    options: Vec<Choice>,
    question_id: Uuid,
    answered_count: u64,
    rows: &[SnapshotRow],
) -> AppResult<Vec<OptionFrequency>> {
    let mut counts = HashMap::<&str, u64>::new();
    for row in rows {
        let Some(value) = row.answers.get(&question_id) else {
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
    Ok(options
        .into_iter()
        .map(|option| {
            let count = counts
                .get(option.id.to_string().as_str())
                .copied()
                .unwrap_or(0);
            OptionFrequency {
                option_id: option.id,
                label: option.label,
                count,
                proportion: (answered_count > 0).then(|| count as f64 / answered_count as f64),
            }
        })
        .collect())
}

fn invalid_stored_answer() -> AppError {
    // 不在诊断信息里拼接答案正文。
    AppError::internal("已保存答案与发布题型不一致")
}

fn update_mean(mean: &mut f64, value: f64, count: u64) {
    let weight = 1.0 / count as f64;
    // 同号时差值不会溢出；异号时改用加权形式，避免 MAX - (-MAX) 溢出。
    if mean.is_sign_positive() == value.is_sign_positive() {
        *mean += (value - *mean) * weight;
    } else {
        *mean = *mean * (1.0 - weight) + value * weight;
    }
}

fn encode_csv(
    document: &Document,
    version_id: Uuid,
    version_number: i32,
    rows: &[SnapshotRow],
) -> AppResult<Vec<u8>> {
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_writer(Vec::new());
    let version_id = version_id.to_string();
    let version_number = version_number.to_string();
    let with_states = document.logic.is_some();
    let mut record: Vec<Cow<'_, str>> =
        Vec::with_capacity(document.questions.len() * if with_states { 2 } else { 1 } + 4);
    record.extend(
        [
            "receipt_id",
            "version_id",
            "version_number",
            "submitted_at (Asia/Shanghai +08:00)",
        ]
        .into_iter()
        .map(Cow::Borrowed),
    );
    for question in &document.questions {
        record.push(Cow::Owned(format!("{} [{}]", question.title, question.id)));
        if with_states {
            record.push(Cow::Owned(format!("状态 [{}]", question.id)));
        }
    }
    for cell in &record {
        writer
            .write_field(formula_safe(cell).as_bytes())
            .map_err(|error| AppError::internal(error))?;
    }
    writer
        .write_record(std::iter::empty::<&[u8]>())
        .map_err(|error| AppError::internal(error))?;
    for SnapshotRow {
        receipt_id,
        submitted_at,
        answers,
        question_states,
    } in rows
    {
        record.clear();
        record.push(Cow::Owned(receipt_id.to_string()));
        record.push(Cow::Borrowed(&version_id));
        record.push(Cow::Borrowed(&version_number));
        record.push(Cow::Owned(
            submitted_at
                .with_timezone(&SHANGHAI_OFFSET)
                .to_rfc3339_opts(SecondsFormat::AutoSi, false),
        ));
        for question in &document.questions {
            let cell = match answers.get(&question.id) {
                None | Some(Value::Null) => Cow::Borrowed(""),
                Some(Value::String(value)) => Cow::Borrowed(value.as_str()),
                // 多选保持 JSON 数组，不用逗号连接，以免丢失集合与 CSV 字段边界。
                Some(value) => Cow::Owned(serde_json::to_string(value)?),
            };
            record.push(cell);
            if with_states {
                let state = match question_states
                    .get(&question.id)
                    .ok_or_else(invalid_stored_answer)?
                {
                    AnswerState::Answered => "已答",
                    AnswerState::Unanswered => "适用未答",
                    AnswerState::Skipped => "规则跳过",
                };
                record.push(Cow::Borrowed(state));
            }
        }
        for cell in &record {
            writer
                .write_field(formula_safe(cell).as_bytes())
                .map_err(|error| AppError::internal(error))?;
        }
        writer
            .write_record(std::iter::empty::<&[u8]>())
            .map_err(|error| AppError::internal(error))?;
    }
    writer
        .into_inner()
        .map_err(|error| AppError::internal(error))
}

fn formula_safe(value: &str) -> Cow<'_, str> {
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
