//! 本人问卷管理：HTTP 函数处理输入输出，下面的业务函数统一负责授权和事务。
//!
//! 草稿与发布版本分别保存；修改草稿不会影响填写者已经读取的题面。

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseTransaction, EntityTrait, FromQueryResult,
    IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait,
    sea_query::Expr,
};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    AppState,
    auth::AuthUser,
    domain::{Document, ValidationError},
    entities::{survey, version},
    error::{AppError, AppResult, ErrorResponse},
    http::{ApiJson, ApiPath, ApiQuery, Page, Pagination},
    repository,
};

pub(crate) use crate::entities::{FillingMode, SurveyStatus};

/// 创建时可以提交空文档；不完整的题面可以保存，但不能发布。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateSurvey {
    pub document: Document,
}

/// 保存整个草稿；题目数组和选项数组的顺序就是显示顺序。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaveDraft {
    /// 必须等于最后一次读取或保存成功返回的修订号。
    #[schema(minimum = 1)]
    pub revision: i64,
    pub document: Document,
}

/// 发布的是服务器上已保存的这一修订，不接受另一个隐式草稿。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublishSurvey {
    #[schema(minimum = 1)]
    pub revision: i64,
}

/// 局部修改回收设置：deadline 省略保留原值，null 清空，时间值替换。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CollectionSettings {
    /// 首次发布前必须选定；发布后只能传原模式，不能更换。
    pub mode: Option<FillingMode>,
    /// RFC 3339 时间，服务端按 UTC 比较；到期后调整不会自动恢复回收。
    #[serde(default, deserialize_with = "deserialize_deadline")]
    #[schema(value_type = Option<DateTime<Utc>>)]
    pub deadline: Option<Option<DateTime<Utc>>>,
}

// 普通 Option 会混淆“没有这个字段”和 null，所以只为 PATCH 的 deadline 保留三态。
fn deserialize_deadline<'de, D>(deserializer: D) -> Result<Option<Option<DateTime<Utc>>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<DateTime<Utc>>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
pub(crate) struct SurveyFilters {
    /// 按草稿标题进行不区分大小写的字面子串筛选。
    pub(crate) title: Option<String>,
    pub(crate) status: Option<SurveyStatus>,
    #[serde(flatten)]
    pub(crate) pagination: Pagination,
}

/// 管理详情不暴露数据库 Entity，也不包含任何答卷或账号认证信息。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct SurveyDetail {
    pub id: Uuid,
    pub status: SurveyStatus,
    pub mode: Option<FillingMode>,
    pub deadline: Option<DateTime<Utc>>,
    pub taken_down: bool,
    pub revision: i64,
    pub current_version_id: Option<Uuid>,
    pub draft: Document,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 发布后提供的同源资源链接；链接永远指向问卷，而不是某个历史版本。
    pub share_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct SurveySummary {
    pub id: Uuid,
    pub title: String,
    pub status: SurveyStatus,
    pub mode: Option<FillingMode>,
    pub deadline: Option<DateTime<Utc>>,
    pub taken_down: bool,
    pub revision: i64,
    pub current_version_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 预览只读取已保存草稿并提供发布校验结果，不保存或提交正式答卷。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct SurveyPreview {
    pub survey_id: Uuid,
    pub revision: i64,
    pub document: Document,
    pub mode: Option<FillingMode>,
    /// 仅表示题面校验通过；实际发布还会检查状态、修订、模式和截止时间。
    pub publishable: bool,
    pub validation_error: Option<ValidationError>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct VersionSummary {
    pub id: Uuid,
    pub number: i32,
    pub published_at: DateTime<Utc>,
    pub is_current: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PublishedSurvey {
    pub survey: SurveyDetail,
    pub version: VersionSummary,
}

// 列表只从 JSONB 中取标题，不把每份问卷的全部题目都加载到内存。
#[derive(FromQueryResult)]
struct SurveyListRow {
    id: Uuid,
    title: String,
    status: SurveyStatus,
    mode: Option<FillingMode>,
    deadline: Option<DateTime<Utc>>,
    taken_down: bool,
    revision: i64,
    current_version_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(FromQueryResult)]
struct VersionListRow {
    id: Uuid,
    number: i32,
    published_at: DateTime<Utc>,
}

pub(crate) fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(detail, delete))
        .routes(routes!(save_draft))
        .routes(routes!(preview))
        .routes(routes!(copy))
        .routes(routes!(publish))
        .routes(routes!(collection))
        .routes(routes!(close))
        .routes(routes!(resume))
        .routes(routes!(archive))
        .routes(routes!(versions))
}

/// 创建本人未发布问卷
#[utoipa::path(
    post, path = "/api/surveys", tag = "问卷管理", request_body = CreateSurvey,
    security(("cookieAuth" = [])),
    responses(
        (status = 201, description = "草稿已创建，revision 从 1 开始", body = SurveyDetail),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 403, description = "请求来源未通过 CSRF 检查", body = ErrorResponse),
        (status = 400, description = "JSON 格式或字段类型错误", body = ErrorResponse),
        (status = 413, description = "请求体超过 1 MiB", body = ErrorResponse),
        (status = 422, description = "输入或草稿基础约束不合法", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    ApiJson(input): ApiJson<CreateSurvey>,
) -> AppResult<(StatusCode, Json<SurveyDetail>)> {
    let result = create_survey(&state, user.id, &headers, input.document).await?;
    Ok((StatusCode::CREATED, Json(result)))
}

/// 分页筛选本人的问卷
///
/// 标题采用字面子串匹配，% 和 _ 不作为通配符。状态筛选包含尚未落库的到期事实。
#[utoipa::path(
    get, path = "/api/surveys", tag = "问卷管理", security(("cookieAuth" = [])),
    params(
        ("title" = Option<String>, Query, description = "草稿标题子串，不区分大小写"),
        ("status" = Option<SurveyStatus>, Query, description = "业务状态，到期即视为 closed"),
        ("page" = Option<u64>, Query, description = "页码，从 1 开始，默认 1"),
        ("page_size" = Option<u64>, Query, description = "每页条数，默认 20，最大 100")
    ),
    responses(
        (status = 200, description = "按更新时间和问卷 ID 倒序的本人问卷", body = Page<SurveySummary>),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 400, description = "查询参数格式错误", body = ErrorResponse),
        (status = 422, description = "筛选或分页参数不合法", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    ApiQuery(filters): ApiQuery<SurveyFilters>,
) -> AppResult<Json<Page<SurveySummary>>> {
    Ok(Json(list_surveys(&state, user.id, filters).await?))
}

/// 查看本人问卷的管理详情和草稿
#[utoipa::path(
    get, path = "/api/surveys/{id}", tag = "问卷管理", security(("cookieAuth" = [])),
    params(("id" = Uuid, Path, description = "本人问卷 ID")),
    responses(
        (status = 200, description = "管理详情；草稿与当前发布版本相互独立", body = SurveyDetail),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 404, description = "资源不存在或不属于当前用户", body = ErrorResponse),
        (status = 400, description = "路径参数格式错误", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn detail(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
) -> AppResult<Json<SurveyDetail>> {
    let record = repository::owned_survey(&state.db, user.id, id).await?;
    Ok(Json(survey_detail(record, Utc::now())?))
}

/// 按修订号保存整个草稿
///
/// 成功后 revision 加一；旧修订返回冲突，不覆盖其他窗口的新内容。已归档问卷不能编辑。
#[utoipa::path(
    put, path = "/api/surveys/{id}/draft", tag = "问卷管理", request_body = SaveDraft,
    security(("cookieAuth" = [])), params(("id" = Uuid, Path, description = "本人问卷 ID")),
    responses(
        (status = 200, description = "已保存；使用返回的新 revision 继续编辑", body = SurveyDetail),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 403, description = "请求来源未通过 CSRF 检查", body = ErrorResponse),
        (status = 404, description = "资源不存在或不属于当前用户", body = ErrorResponse),
        (status = 409, description = "修订冲突或问卷已归档", body = ErrorResponse),
        (status = 400, description = "路径或 JSON 格式错误", body = ErrorResponse),
        (status = 413, description = "请求体超过 1 MiB", body = ErrorResponse),
        (status = 422, description = "输入或草稿基础约束不合法", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn save_draft(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<SaveDraft>,
) -> AppResult<Json<SurveyDetail>> {
    Ok(Json(
        save_survey_draft(&state, user.id, &headers, id, input).await?,
    ))
}

/// 预览已保存草稿，不产生正式答卷
///
/// 返回的纯文本题面可用于电脑和手机布局预览。发布校验失败仍返回草稿和可定位的错误。
#[utoipa::path(
    get, path = "/api/surveys/{id}/preview", tag = "问卷管理", security(("cookieAuth" = [])),
    params(("id" = Uuid, Path, description = "本人问卷 ID")),
    responses(
        (status = 200, description = "已保存的预览内容及发布校验结果", body = SurveyPreview),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 404, description = "资源不存在或不属于当前用户", body = ErrorResponse),
        (status = 400, description = "路径参数格式错误", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn preview(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
) -> AppResult<Json<SurveyPreview>> {
    let record = repository::owned_survey(&state.db, user.id, id).await?;
    let document: Document = serde_json::from_value(record.draft)?;
    let validation_error = document.validate_publish().err();
    Ok(Json(SurveyPreview {
        survey_id: record.id,
        revision: record.revision,
        document,
        mode: record.mode,
        publishable: validation_error.is_none(),
        validation_error,
    }))
}

/// 复制本人问卷为独立的新草稿
///
/// 归档问卷取最后一个发布版本，其他问卷取已保存草稿。所有题目和选项重新生成 ID，
/// 不复制回收设置、发布状态、答卷或限次资格。
#[utoipa::path(
    post, path = "/api/surveys/{id}/copy", tag = "问卷管理", security(("cookieAuth" = [])),
    params(("id" = Uuid, Path, description = "本人来源问卷 ID")),
    responses(
        (status = 201, description = "新的未发布问卷，revision 为 1", body = SurveyDetail),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 403, description = "请求来源未通过 CSRF 检查", body = ErrorResponse),
        (status = 404, description = "资源不存在或不属于当前用户", body = ErrorResponse),
        (status = 400, description = "路径参数格式错误", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn copy(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    ApiPath(id): ApiPath<Uuid>,
) -> AppResult<(StatusCode, Json<SurveyDetail>)> {
    let result = copy_survey(&state, user.id, &headers, id).await?;
    Ok((StatusCode::CREATED, Json(result)))
}

/// 彻底删除本人从未发布过的问卷
#[utoipa::path(
    delete, path = "/api/surveys/{id}", tag = "问卷管理", security(("cookieAuth" = [])),
    params(("id" = Uuid, Path, description = "本人未发布问卷 ID")),
    responses(
        (status = 204, description = "已删除，无响应体"),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 403, description = "请求来源未通过 CSRF 检查", body = ErrorResponse),
        (status = 404, description = "资源不存在或不属于当前用户", body = ErrorResponse),
        (status = 409, description = "发布过的问卷只能归档", body = ErrorResponse),
        (status = 400, description = "路径参数格式错误", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    ApiPath(id): ApiPath<Uuid>,
) -> AppResult<StatusCode> {
    delete_survey(&state, user.id, &headers, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 发布指定修订的不可变内容快照
///
/// 首次发布前须设置填写模式且截止时间未到期。重新发布只切换当前版本；
/// 关闭状态不会被发布操作恢复，历史版本和历史答卷始终保留。
/// 草稿未变且内容与当前版本完全一致时按幂等重放返回现有版本，不新增版本号；
/// 重放判定先于当前内容上限校验，历史版本未改动内容仍可重复发布。
#[utoipa::path(
    post, path = "/api/surveys/{id}/publish", tag = "问卷管理", request_body = PublishSurvey,
    security(("cookieAuth" = [])), params(("id" = Uuid, Path, description = "本人问卷 ID")),
    responses(
        (status = 201, description = "新版本已创建并成为当前版本", body = PublishedSurvey),
        (status = 200, description = "内容与当前版本一致：未新增版本，返回当前版本", body = PublishedSurvey),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 403, description = "请求来源未通过 CSRF 检查", body = ErrorResponse),
        (status = 404, description = "资源不存在或不属于当前用户", body = ErrorResponse),
        (status = 409, description = "修订冲突、已归档、未选模式或首次发布时已到期", body = ErrorResponse),
        (status = 400, description = "路径或 JSON 格式错误", body = ErrorResponse),
        (status = 413, description = "请求体超过 1 MiB", body = ErrorResponse),
        (status = 422, description = "完整题面校验失败，或最坏情况答卷超出单次提交上限", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn publish(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<PublishSurvey>,
) -> AppResult<(StatusCode, Json<PublishedSurvey>)> {
    let (status, result) = publish_survey(&state, user.id, &headers, id, input.revision).await?;
    Ok((status, Json(result)))
}

/// 设置填写模式或调整截止时间
///
/// 先记录旧截止时间已经到期的事实，再应用新截止时间；延后或清空截止不会自动恢复回收。
/// 填写模式在首次发布后永久冻结。
#[utoipa::path(
    patch, path = "/api/surveys/{id}/collection", tag = "问卷管理", request_body = CollectionSettings,
    security(("cookieAuth" = [])), params(("id" = Uuid, Path, description = "本人问卷 ID")),
    responses(
        (status = 200, description = "回收设置已更新；检查返回的业务状态", body = SurveyDetail),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 403, description = "请求来源未通过 CSRF 检查", body = ErrorResponse),
        (status = 404, description = "资源不存在或不属于当前用户", body = ErrorResponse),
        (status = 409, description = "已归档或试图更改已冻结的填写模式", body = ErrorResponse),
        (status = 400, description = "路径或 JSON 格式错误", body = ErrorResponse),
        (status = 413, description = "请求体超过 1 MiB", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn collection(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<CollectionSettings>,
) -> AppResult<Json<SurveyDetail>> {
    Ok(Json(
        update_collection(&state, user.id, &headers, id, input).await?,
    ))
}

/// 主动关闭回收
///
/// 与提交使用同一问卷行锁；关闭完成后发起的新提交不能成功。重复关闭不重新开放问卷。
#[utoipa::path(
    post, path = "/api/surveys/{id}/close", tag = "问卷管理", security(("cookieAuth" = [])),
    params(("id" = Uuid, Path, description = "本人问卷 ID")),
    responses(
        (status = 200, description = "回收已关闭", body = SurveyDetail),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 403, description = "请求来源未通过 CSRF 检查", body = ErrorResponse),
        (status = 404, description = "资源不存在或不属于当前用户", body = ErrorResponse),
        (status = 409, description = "未发布或已归档", body = ErrorResponse),
        (status = 400, description = "路径参数格式错误", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn close(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    ApiPath(id): ApiPath<Uuid>,
) -> AppResult<Json<SurveyDetail>> {
    Ok(Json(
        change_status(&state, user.id, &headers, id, CollectionAction::Close).await?,
    ))
}

/// 显式恢复已关闭问卷的回收状态
///
/// 截止时间必须为空或晚于服务端当前时间。本操作不清除管理员下架标志。
#[utoipa::path(
    post, path = "/api/surveys/{id}/resume", tag = "问卷管理", security(("cookieAuth" = [])),
    params(("id" = Uuid, Path, description = "本人问卷 ID")),
    responses(
        (status = 200, description = "业务状态恢复为 collecting；下架标志仍独立生效", body = SurveyDetail),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 403, description = "请求来源未通过 CSRF 检查", body = ErrorResponse),
        (status = 404, description = "资源不存在或不属于当前用户", body = ErrorResponse),
        (status = 409, description = "不是已关闭问卷、已归档或截止时间已到", body = ErrorResponse),
        (status = 400, description = "路径参数格式错误", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn resume(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    ApiPath(id): ApiPath<Uuid>,
) -> AppResult<Json<SurveyDetail>> {
    Ok(Json(
        change_status(&state, user.id, &headers, id, CollectionAction::Resume).await?,
    ))
}

/// 归档已经发布过的问卷
///
/// 归档不可撤销；禁止继续编辑、发布或恢复，但保留历史版本、答卷、统计及复制能力。
#[utoipa::path(
    post, path = "/api/surveys/{id}/archive", tag = "问卷管理", security(("cookieAuth" = [])),
    params(("id" = Uuid, Path, description = "本人问卷 ID")),
    responses(
        (status = 200, description = "已归档", body = SurveyDetail),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 403, description = "请求来源未通过 CSRF 检查", body = ErrorResponse),
        (status = 404, description = "资源不存在或不属于当前用户", body = ErrorResponse),
        (status = 409, description = "未发布问卷应使用删除接口", body = ErrorResponse),
        (status = 400, description = "路径参数格式错误", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn archive(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    ApiPath(id): ApiPath<Uuid>,
) -> AppResult<Json<SurveyDetail>> {
    Ok(Json(
        change_status(&state, user.id, &headers, id, CollectionAction::Archive).await?,
    ))
}

/// 分页列出本人问卷的发布版本
///
/// 按版本号倒序返回，已归档问卷仍可查询；历史版本仅供授权查看和统计，不能继续收集答卷。
#[utoipa::path(
    get, path = "/api/surveys/{id}/versions", tag = "问卷管理", security(("cookieAuth" = [])),
    params(
        ("id" = Uuid, Path, description = "本人问卷 ID"),
        ("page" = Option<u64>, Query, description = "页码，从 1 开始，默认 1"),
        ("page_size" = Option<u64>, Query, description = "每页条数，默认 20，最大 100")
    ),
    responses(
        (status = 200, description = "发布版本元数据和当前版本标志", body = Page<VersionSummary>),
        (status = 401, description = "未登录或账号不可用", body = ErrorResponse),
        (status = 404, description = "资源不存在或不属于当前用户", body = ErrorResponse),
        (status = 400, description = "路径或查询参数格式错误", body = ErrorResponse),
        (status = 422, description = "路径或分页参数不合法", body = ErrorResponse),
        (status = 500, description = "服务端错误", body = ErrorResponse)
    )
)]
async fn versions(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(pagination): ApiQuery<Pagination>,
) -> AppResult<Json<Page<VersionSummary>>> {
    Ok(Json(list_versions(&state, user.id, id, pagination).await?))
}

// 以下是业务层。所有写入只使用同一个 tx，绝不在事务中绕回连接池执行写入。
// 写入函数接收原请求头，只用于持锁后重验 Session；授权主体仍是 owner_id。

async fn create_survey(
    state: &AppState,
    owner_id: Uuid,
    headers: &HeaderMap,
    document: Document,
) -> AppResult<SurveyDetail> {
    document.validate_draft()?;
    let tx = state.db.begin().await?;
    repository::lock_session_user(&tx, headers, owner_id).await?;
    let now = Utc::now();
    let record = insert_draft(&tx, owner_id, document, now).await?;
    let result = survey_detail(record, now)?;
    tx.commit().await?;
    Ok(result)
}

async fn insert_draft(
    tx: &DatabaseTransaction,
    owner_id: Uuid,
    document: Document,
    now: DateTime<Utc>,
) -> AppResult<survey::Model> {
    Ok(survey::ActiveModel {
        id: Set(Uuid::new_v4()),
        owner_id: Set(owner_id),
        status: Set(SurveyStatus::Unpublished),
        mode: Set(None),
        deadline: Set(None),
        taken_down: Set(false),
        draft: Set(serde_json::to_value(document)?),
        revision: Set(1),
        current_version_id: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(tx)
    .await?)
}

async fn list_surveys(
    state: &AppState,
    owner_id: Uuid,
    filters: SurveyFilters,
) -> AppResult<Page<SurveySummary>> {
    let bounds = filters.pagination.bounds()?;
    let now = Utc::now();
    let mut query = survey::Entity::find().filter(survey::Column::OwnerId.eq(owner_id));
    if let Some(title) = filters.title {
        // strpos 是字面子串搜索：参数仍绑定，不拼接 SQL，也不存在 LIKE 通配符绕过。
        query = query.filter(Expr::cust_with_values(
            "strpos(lower(\"surveys\".\"draft\" ->> 'title'), lower($1)) > 0",
            [title],
        ));
    }
    if let Some(status) = filters.status {
        let condition = match status {
            SurveyStatus::Collecting => Condition::all()
                .add(survey::Column::Status.eq(SurveyStatus::Collecting))
                .add(
                    Condition::any()
                        .add(survey::Column::Deadline.is_null())
                        .add(survey::Column::Deadline.gt(now)),
                ),
            SurveyStatus::Closed => Condition::any()
                .add(survey::Column::Status.eq(SurveyStatus::Closed))
                .add(
                    Condition::all()
                        .add(survey::Column::Status.eq(SurveyStatus::Collecting))
                        .add(survey::Column::Deadline.lte(now)),
                ),
            SurveyStatus::Unpublished | SurveyStatus::Archived => {
                Condition::all().add(survey::Column::Status.eq(status))
            }
        };
        query = query.filter(condition);
    }
    let tx = repository::read_snapshot(&state.db).await?;
    let total = query.clone().count(&tx).await?;
    let rows = query
        .select_only()
        .columns([
            survey::Column::Id,
            survey::Column::Status,
            survey::Column::Mode,
            survey::Column::Deadline,
            survey::Column::TakenDown,
            survey::Column::Revision,
            survey::Column::CurrentVersionId,
            survey::Column::CreatedAt,
            survey::Column::UpdatedAt,
        ])
        .column_as(repository::title_column(), "title")
        .order_by_desc(survey::Column::UpdatedAt)
        .order_by_desc(survey::Column::Id)
        .offset(bounds.offset)
        .limit(bounds.limit)
        .into_model::<SurveyListRow>()
        .all(&tx)
        .await?;
    let items = rows
        .into_iter()
        .map(|row| SurveySummary {
            id: row.id,
            title: row.title,
            status: row.status.effective(row.deadline, now),
            mode: row.mode,
            deadline: row.deadline,
            taken_down: row.taken_down,
            revision: row.revision,
            current_version_id: row.current_version_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect();
    tx.commit().await?;
    Ok(Page::new(items, total, bounds))
}

async fn save_survey_draft(
    state: &AppState,
    owner_id: Uuid,
    headers: &HeaderMap,
    id: Uuid,
    input: SaveDraft,
) -> AppResult<SurveyDetail> {
    input.document.validate_draft()?;
    let tx = state.db.begin().await?;
    let record = repository::lock_owned_survey(&tx, headers, owner_id, id).await?;
    let now = Utc::now();
    let record = repository::expire_survey(&tx, record, now).await?;
    ensure_editable(&record)?;
    ensure_revision(&record, input.revision)?;
    let next_revision = record
        .revision
        .checked_add(1)
        .ok_or_else(|| AppError::internal("草稿修订号溢出"))?;
    let mut active = record.into_active_model();
    active.draft = Set(serde_json::to_value(input.document)?);
    active.revision = Set(next_revision);
    active.updated_at = Set(now);
    let result = survey_detail(active.update(&tx).await?, now)?;
    tx.commit().await?;
    Ok(result)
}

async fn copy_survey(
    state: &AppState,
    owner_id: Uuid,
    headers: &HeaderMap,
    id: Uuid,
) -> AppResult<SurveyDetail> {
    let tx = state.db.begin().await?;
    let source = repository::lock_owned_survey(&tx, headers, owner_id, id).await?;
    let source_document = if source.status == SurveyStatus::Archived {
        let version_id = source
            .current_version_id
            .ok_or_else(|| AppError::internal("归档问卷缺少发布版本"))?;
        version::Entity::find_by_id(version_id)
            .one(&tx)
            .await?
            .ok_or_else(|| AppError::internal("归档问卷缺少发布版本"))?
            .document
    } else {
        source.draft
    };
    let mut document: Document = serde_json::from_value(source_document)?;
    document.renew_ids();
    document.validate_draft()?;
    let now = Utc::now();
    let record = insert_draft(&tx, owner_id, document, now).await?;
    let result = survey_detail(record, now)?;
    tx.commit().await?;
    Ok(result)
}

async fn delete_survey(
    state: &AppState,
    owner_id: Uuid,
    headers: &HeaderMap,
    id: Uuid,
) -> AppResult<()> {
    let tx = state.db.begin().await?;
    let record = repository::lock_owned_survey(&tx, headers, owner_id, id).await?;
    if record.status != SurveyStatus::Unpublished {
        return Err(AppError::conflict(
            "survey_published",
            "发布过的问卷不能删除，请使用归档",
        ));
    }
    survey::Entity::delete_by_id(id).exec(&tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn publish_survey(
    state: &AppState,
    owner_id: Uuid,
    headers: &HeaderMap,
    id: Uuid,
    revision: i64,
) -> AppResult<(StatusCode, PublishedSurvey)> {
    let tx = state.db.begin().await?;
    let record = repository::lock_owned_survey(&tx, headers, owner_id, id).await?;
    let now = Utc::now();
    let record = repository::expire_survey(&tx, record, now).await?;
    ensure_editable(&record)?;
    ensure_revision(&record, revision)?;

    // 先按当前规则校验这版内容，再判断是否为幂等重放。
    let document = Document::deserialize(&record.draft)?;
    document.validate_publish()?;
    if record.mode.is_none() {
        return Err(AppError::conflict("mode_required", "请先设置填写模式"));
    }

    // 内容与当前版本完全一致的重复发布（例如响应丢失后的重试）是幂等重放：
    // 不新增版本，直接返回当前版本，避免同一份题面出现两个版本号。
    if let Some(current_id) = record.current_version_id {
        let current = version::Entity::find_by_id(current_id)
            .one(&tx)
            .await?
            .ok_or_else(|| AppError::internal("current version is missing"))?;
        if current.document == record.draft {
            let result = PublishedSurvey {
                survey: survey_detail(record, now)?,
                version: VersionSummary {
                    id: current.id,
                    number: current.number,
                    published_at: current.published_at,
                    is_current: true,
                },
            };
            tx.commit().await?;
            return Ok((StatusCode::OK, result));
        }
    }

    let first_publish = record.current_version_id.is_none();
    if first_publish {
        ensure_future_deadline(record.deadline, now)?;
    }
    let previous_number = version::Entity::find()
        .filter(version::Column::SurveyId.eq(id))
        .select_only()
        .column(version::Column::Number)
        .order_by_desc(version::Column::Number)
        .into_tuple::<i32>()
        .one(&tx)
        .await?
        .unwrap_or(0);
    let number = previous_number
        .checked_add(1)
        .ok_or_else(|| AppError::internal("发布版本号溢出"))?;
    let version_id = Uuid::new_v4();
    version::Entity::insert(version::ActiveModel {
        id: Set(version_id),
        survey_id: Set(id),
        number: Set(number),
        document: Set(record.draft),
        published_at: Set(now),
    })
    .exec(&tx)
    .await?;
    // 只首次发布进入回收中。后续发布沿用刚刚物化的 closed/collecting 状态。
    let updated = survey::ActiveModel {
        id: Set(id),
        status: Set(if first_publish {
            SurveyStatus::Collecting
        } else {
            record.status
        }),
        current_version_id: Set(Some(version_id)),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(&tx)
    .await?;
    let result = PublishedSurvey {
        survey: survey_detail(updated, now)?,
        version: VersionSummary {
            id: version_id,
            number,
            published_at: now,
            is_current: true,
        },
    };
    tx.commit().await?;
    Ok((StatusCode::CREATED, result))
}

async fn update_collection(
    state: &AppState,
    owner_id: Uuid,
    headers: &HeaderMap,
    id: Uuid,
    input: CollectionSettings,
) -> AppResult<SurveyDetail> {
    let tx = state.db.begin().await?;
    let record = repository::lock_owned_survey(&tx, headers, owner_id, id).await?;
    let now = Utc::now();
    // 必须在覆盖 deadline 之前执行，否则延期会抹掉“已经到期”的事实。
    let record = repository::expire_survey(&tx, record, now).await?;
    ensure_editable(&record)?;
    validate_mode_change(record.current_version_id, record.mode, input.mode)?;
    let deadline_changed = input.deadline.is_some();
    let mut active = record.into_active_model();
    if let Some(mode) = input.mode {
        active.mode = Set(Some(mode));
    }
    if let Some(deadline) = input.deadline {
        active.deadline = Set(deadline);
    }
    active.updated_at = Set(now);
    let updated = active.update(&tx).await?;
    // 只有写入新截止时间时才可能产生新到期；它永远不会把 closed 改成 collecting。
    let updated = if deadline_changed {
        repository::expire_survey(&tx, updated, now).await?
    } else {
        updated
    };
    let result = survey_detail(updated, now)?;
    tx.commit().await?;
    Ok(result)
}

#[derive(Clone, Copy)]
pub(crate) enum CollectionAction {
    Close,
    Resume,
    Archive,
}

async fn change_status(
    state: &AppState,
    owner_id: Uuid,
    headers: &HeaderMap,
    id: Uuid,
    action: CollectionAction,
) -> AppResult<SurveyDetail> {
    let tx = state.db.begin().await?;
    let record = repository::lock_owned_survey(&tx, headers, owner_id, id).await?;
    let now = Utc::now();
    let record = repository::expire_survey(&tx, record, now).await?;
    let current = record.status;
    let target = next_collection_status(action, current, record.deadline, now)?;
    let updated = if target != current {
        let mut active = record.into_active_model();
        active.status = Set(target);
        active.updated_at = Set(now);
        active.update(&tx).await?
    } else {
        record
    };
    let result = survey_detail(updated, now)?;
    tx.commit().await?;
    Ok(result)
}

async fn list_versions(
    state: &AppState,
    owner_id: Uuid,
    id: Uuid,
    pagination: Pagination,
) -> AppResult<Page<VersionSummary>> {
    let bounds = pagination.bounds()?;
    let tx = repository::read_snapshot(&state.db).await?;
    let meta = repository::owned_survey_meta(&tx, owner_id, id).await?;
    let query = version::Entity::find().filter(version::Column::SurveyId.eq(meta.id));
    let total = query.clone().count(&tx).await?;
    let rows = query
        .select_only()
        .columns([
            version::Column::Id,
            version::Column::Number,
            version::Column::PublishedAt,
        ])
        .order_by_desc(version::Column::Number)
        .offset(bounds.offset)
        .limit(bounds.limit)
        .into_model::<VersionListRow>()
        .all(&tx)
        .await?;
    let items = rows
        .into_iter()
        .map(|row| VersionSummary {
            id: row.id,
            number: row.number,
            published_at: row.published_at,
            is_current: meta.current_version_id == Some(row.id),
        })
        .collect();
    tx.commit().await?;
    Ok(Page::new(items, total, bounds))
}

/// 纯状态转移：动作与当前状态的组合决定目标状态或冲突错误，不读写数据库。
///
/// 归档优先于其它规则；未发布问卷不能关闭、恢复或归档；恢复回收还要求截止时间未到。
pub(crate) fn next_collection_status(
    action: CollectionAction,
    current: SurveyStatus,
    deadline: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> AppResult<SurveyStatus> {
    match (action, current) {
        (
            CollectionAction::Archive,
            SurveyStatus::Collecting | SurveyStatus::Closed | SurveyStatus::Archived,
        ) => Ok(SurveyStatus::Archived),
        (_, SurveyStatus::Archived) => Err(AppError::conflict(
            "survey_archived",
            "已归档问卷不能更改状态",
        )),
        (_, SurveyStatus::Unpublished) => {
            Err(AppError::conflict("survey_not_published", "问卷尚未发布"))
        }
        (CollectionAction::Close, SurveyStatus::Collecting | SurveyStatus::Closed) => {
            Ok(SurveyStatus::Closed)
        }
        (CollectionAction::Resume, SurveyStatus::Closed) => {
            ensure_future_deadline(deadline, now)?;
            Ok(SurveyStatus::Collecting)
        }
        (CollectionAction::Resume, SurveyStatus::Collecting) => Err(AppError::conflict(
            "survey_not_closed",
            "只有已关闭问卷可以恢复回收",
        )),
    }
}

/// 纯模式冻结检查：首次发布之后只能在请求中重复原模式。
pub(crate) fn validate_mode_change(
    current_version_id: Option<Uuid>,
    current_mode: Option<FillingMode>,
    requested_mode: Option<FillingMode>,
) -> AppResult<()> {
    if let Some(mode) = requested_mode
        && current_version_id.is_some()
        && current_mode != Some(mode)
    {
        return Err(AppError::conflict(
            "mode_frozen",
            "首次发布后不能更改填写模式",
        ));
    }
    Ok(())
}

pub(crate) fn ensure_editable(record: &survey::Model) -> AppResult<()> {
    if record.status == SurveyStatus::Archived {
        return Err(AppError::conflict(
            "survey_archived",
            "已归档问卷不能编辑或发布，可以复制为新问卷",
        ));
    }
    Ok(())
}

pub(crate) fn ensure_revision(record: &survey::Model, revision: i64) -> AppResult<()> {
    if record.revision != revision {
        return Err(AppError::conflict(
            "revision_conflict",
            "草稿已在其他窗口修改，请重新读取后核对内容",
        ));
    }
    Ok(())
}

pub(crate) fn ensure_future_deadline(
    deadline: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if deadline.is_some_and(|deadline| deadline <= now) {
        return Err(AppError::conflict(
            "deadline_expired",
            "截止时间已到，请先调整截止时间",
        ));
    }
    Ok(())
}

pub(crate) fn survey_detail(record: survey::Model, now: DateTime<Utc>) -> AppResult<SurveyDetail> {
    Ok(SurveyDetail {
        id: record.id,
        status: record.status.effective(record.deadline, now),
        mode: record.mode,
        deadline: record.deadline,
        taken_down: record.taken_down,
        revision: record.revision,
        current_version_id: record.current_version_id,
        draft: serde_json::from_value(record.draft)?,
        created_at: record.created_at,
        updated_at: record.updated_at,
        share_url: record
            .current_version_id
            .map(|_| format!("/api/public/surveys/{}", record.id)),
    })
}
