//! 公开填写入口：题面只读与新答卷共用回收资格检查，成功重试不重新占用填写次数。

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::{Uuid, Variant, Version};

use crate::{
    AppState, auth,
    domain::{Answer, Document},
    entities::{response, survey, version},
    error::{AppError, AppResult, ErrorResponse},
    http::{ApiJson, ApiPath},
    repository,
};

/// 公开题面只含当前发布内容及知情提示，不包含发布者账号或草稿。
#[derive(Serialize, ToSchema)]
pub(crate) struct PublicSurvey {
    pub survey_id: Uuid,
    pub version_id: Uuid,
    pub version_number: i32,
    /// anonymous 为公开匿名；login_once 为同一账号跨版本仅提交一次。
    #[schema(value_type = crate::surveys::FillingMode)]
    pub mode: String,
    pub deadline: Option<DateTime<Utc>>,
    /// purpose 是发布者说明的收集目的；文本只能作为普通文本展示。
    pub document: Document,
    pub privacy: PrivacyNotice,
}

/// 填写前应展示这些提示，不将登录限次误称为完全匿名。
#[derive(Serialize, ToSchema)]
pub(crate) struct PrivacyNotice {
    pub mode_notice: &'static str,
    pub data_visibility: &'static str,
    pub sensitive_data_notice: &'static str,
    pub public_link_notice: &'static str,
}

/// 一个逻辑提交只生成一次 UUIDv4 秘密；网络重试必须保留原秘密与输入。
/// 不实现 Debug，避免排错时顺手输出答案正文或提交秘密。
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubmitRequest {
    pub version_id: Uuid,
    /// 使用安全随机 UUIDv4，例如浏览器 crypto.randomUUID()；不要放进 URL 或日志。
    pub idempotency_key: Uuid,
    pub answers: Vec<Answer>,
}

/// 新提交与重试都只返回这两个字段，不回传答案、账号或提交秘密。
#[derive(Serialize, ToSchema)]
pub(crate) struct SubmissionReceipt {
    pub status: SubmissionStatus,
    pub receipt_id: Uuid,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubmissionStatus {
    Submitted,
}

// 把锁及其读取结果一起交给业务函数，避免取得锁后误用连接池执行查询。
struct LockedSurvey {
    tx: DatabaseTransaction,
    survey: survey::Model,
    respondent_id: Option<Uuid>,
    owner_disabled: bool,
}

#[derive(Serialize)]
struct NormalizedPayload<'a> {
    version_id: Uuid,
    answers: &'a BTreeMap<Uuid, Value>,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/public/surveys/{id}", get(public_survey))
        .route("/api/public/surveys/{id}/responses", post(submit))
}

/// 读取当前可填写题面。
///
/// anonymous 完全忽略 Cookie；login_once 要求有效会话且尚未提交。
/// 已关闭、到期、下架或发布者禁用时不返回题面。隐私提示与 purpose 应在填写前展示。
#[utoipa::path(
    get,
    path = "/api/public/surveys/{id}",
    tag = "公开填写",
    params(("id" = Uuid, Path, description = "问卷编号，不是发布版本编号")),
    security((), ("cookieAuth" = [])),
    responses(
        (status = 200, description = "当前可填写版本及隐私提示", body = PublicSurvey),
        (status = 401, description = "登录限次模式的会话无效或账号禁用", body = ErrorResponse),
        (status = 404, description = "问卷不存在或不可访问", body = ErrorResponse),
        (status = 409, description = "回收不可用、填写模式已变或该账号已经提交", body = ErrorResponse),
        (status = 400, description = "路径参数无效：invalid_path", body = ErrorResponse),
        (status = 500, description = "内部错误，不暴露数据库细节", body = ErrorResponse)
    )
)]
async fn public_survey(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<PublicSurvey>> {
    read_public_survey(&state, &headers, id).await.map(Json)
}

/// 提交完整答卷或重试同一次提交。
///
/// 首次成功返回 201，合法重试返回 200 和原回执。相同作用域中改变版本或规范化答案返回
/// idempotency_conflict。题目顺序及多选集合顺序不影响幂等性，但重复题目和选项仍会拒绝。
/// 成功后的原载荷重试可跨关闭、下架、发布者禁用及版本切换；login_once 仍须验证填写者身份。
/// 没有成功记录的旧版本提交返回 version_changed；未知版本和非法答案返回 validation_failed。
#[utoipa::path(
    post,
    path = "/api/public/surveys/{id}/responses",
    tag = "公开填写",
    params(("id" = Uuid, Path, description = "问卷编号")),
    request_body = SubmitRequest,
    security((), ("cookieAuth" = [])),
    responses(
        (status = 201, description = "答卷及幂等记录已原子提交", body = SubmissionReceipt),
        (status = 200, description = "相同规范化载荷的成功重试，返回原回执", body = SubmissionReceipt),
        (status = 400, description = "路径参数或 JSON 结构无效：invalid_path、invalid_json", body = ErrorResponse),
        (status = 401, description = "登录限次模式的会话无效或账号禁用", body = ErrorResponse),
        (status = 403, description = "跨站状态变更被拒绝", body = ErrorResponse),
        (status = 404, description = "问卷不存在或不可访问", body = ErrorResponse),
        (status = 409, description = "idempotency_conflict、version_changed、already_submitted 或回收不可用", body = ErrorResponse),
        (status = 413, description = "请求体超过限制", body = ErrorResponse),
        (status = 422, description = "非 UUIDv4 秘密、未知版本或答案校验失败；题目错误含 question_id", body = ErrorResponse),
        (status = 429, description = "提交频率超限", body = ErrorResponse),
        (status = 500, description = "内部错误；未成功提交时不会占用填写次数", body = ErrorResponse)
    )
)]
async fn submit(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<SubmitRequest>,
) -> AppResult<(StatusCode, Json<SubmissionReceipt>)> {
    let (status, receipt) = submit_response(&state, &headers, id, request).await?;
    Ok((status, Json(receipt)))
}

async fn read_public_survey(
    state: &AppState,
    headers: &HeaderMap,
    id: Uuid,
) -> AppResult<PublicSurvey> {
    let locked = lock_public_survey(state, headers, id).await?;
    ensure_available(&locked, Utc::now())?;
    ensure_not_submitted(&locked).await?;
    let version_id = locked
        .survey
        .current_version_id
        .ok_or_else(|| AppError::internal("collecting survey has no version"))?;
    let published = version::Entity::find_by_id(version_id)
        .filter(version::Column::SurveyId.eq(id))
        .one(&locked.tx)
        .await?
        .ok_or_else(|| AppError::internal("current version is missing"))?;
    let mode = locked
        .survey
        .mode
        .ok_or_else(|| AppError::internal("published survey has no mode"))?;
    let mode_notice = match mode.as_str() {
        "anonymous" => {
            "公开匿名：即使已登录，答卷也不关联账号。不承诺按自然人限次，也不宣称不可追踪或严格一人一票。"
        }
        "login_once" => {
            "登录限次：同一账号对本问卷跨版本最多提交一次。平台保留内部账号关联用于限次，不能称为完全匿名；发布者页面及导出不提供账号信息。"
        }
        _ => return Err(AppError::internal("unknown collection mode")),
    };
    let document = serde_json::from_value(published.document)?;
    locked.tx.commit().await?;
    Ok(PublicSurvey {
        survey_id: id,
        version_id,
        version_number: published.number,
        mode,
        deadline: locked.survey.deadline,
        document,
        privacy: PrivacyNotice {
            mode_notice,
            data_visibility: "答卷、统计和导出仅向本问卷发布者开放，不默认公开结果；管理员治理接口不提供他人答卷。数据库运维权限与业务角色不同。",
            sensitive_data_notice: "请阅读调查目的，仅提交调查所需的非敏感信息，不要填写密码、证件号码等不必要的敏感信息。当前系统用于教学演示及知情同意的非敏感调查。",
            public_link_notice: "公开链接不是保密访问控制，获得链接的人可能访问题面。系统不采集设备指纹或地理位置进入答卷，匿名不意味着数据库运维人员无法接触数据。",
        },
    })
}

async fn submit_response(
    state: &AppState,
    headers: &HeaderMap,
    id: Uuid,
    request: SubmitRequest,
) -> AppResult<(StatusCode, SubmissionReceipt)> {
    if request.idempotency_key.get_version() != Some(Version::Random)
        || request.idempotency_key.get_variant() != Variant::RFC4122
    {
        return Err(AppError::validation(
            "idempotency_key 必须是安全随机生成的 UUIDv4",
        ));
    }
    // 只保存秘密的 SHA-256；UUID 的文本大小写、连字符格式不会改变其身份。
    let key_hash = hex::encode(Sha256::digest(request.idempotency_key.as_bytes()));
    let locked = lock_public_survey(state, headers, id).await?;
    let scope = locked
        .respondent_id
        .map_or_else(|| "anonymous".to_owned(), |id| id.to_string());
    let previous = response::Entity::find()
        .select_only()
        .column(response::Column::Id)
        .column(response::Column::VersionId)
        .column(response::Column::PayloadHash)
        .filter(response::Column::SurveyId.eq(id))
        .filter(response::Column::Scope.eq(&scope))
        .filter(response::Column::KeyHash.eq(&key_hash))
        .into_tuple::<(Uuid, Uuid, String)>()
        .one(&locked.tx)
        .await?;
    if let Some((_, original_version, _)) = &previous
        && *original_version != request.version_id
    {
        return Err(idempotency_conflict());
    }

    let submitted_at = Utc::now();
    if previous.is_none() {
        // 必须在取得问卷锁之后读取时间；成功重试不再检查新提交的回收资格。
        ensure_available(&locked, submitted_at)?;
        ensure_not_submitted(&locked).await?;
    }
    let published = version::Entity::find_by_id(request.version_id)
        .filter(version::Column::SurveyId.eq(id))
        .one(&locked.tx)
        .await?
        .ok_or_else(|| AppError::validation("version_id 不属于本问卷的已发布版本"))?;
    if previous.is_none() && locked.survey.current_version_id != Some(request.version_id) {
        return Err(AppError::conflict(
            "version_changed",
            "问卷已更新，请保留当前输入并重新加载题面核对",
        ));
    }

    // 历史版本不可变：旧版本的成功重试仍按旧规则规范化，而不是套用新题面。
    let document: Document = serde_json::from_value(published.document)?;
    let answers = document.validate_answers(request.answers)?;
    let payload = NormalizedPayload {
        version_id: request.version_id,
        answers: &answers,
    };
    let payload_hash = hex::encode(Sha256::digest(serde_json::to_vec(&payload)?));
    if let Some((receipt_id, _, original_hash)) = previous {
        if original_hash != payload_hash {
            return Err(idempotency_conflict());
        }
        locked.tx.commit().await?;
        return Ok((
            StatusCode::OK,
            SubmissionReceipt {
                status: SubmissionStatus::Submitted,
                receipt_id,
            },
        ));
    }

    let receipt_id = Uuid::new_v4();
    // 一条记录同时存完整答案与幂等指纹；数据库还约束同问卷账号唯一，跨版本同样限次。
    // 不使用 upsert：失败不得覆盖答案、占用次数或修改已有成功记录。
    response::Entity::insert(response::ActiveModel {
        id: Set(receipt_id),
        survey_id: Set(id),
        version_id: Set(request.version_id),
        respondent_id: Set(locked.respondent_id),
        scope: Set(scope),
        key_hash: Set(key_hash),
        payload_hash: Set(payload_hash),
        answers: Set(serde_json::to_value(answers)?),
        submitted_at: Set(submitted_at),
    })
    .exec_without_returning(&locked.tx)
    .await?;
    locked.tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        SubmissionReceipt {
            status: SubmissionStatus::Submitted,
            receipt_id,
        },
    ))
}

async fn lock_public_survey(
    state: &AppState,
    headers: &HeaderMap,
    id: Uuid,
) -> AppResult<LockedSurvey> {
    // 锁之前只读归属和模式以决定是否认证，匿名路径绝不解释 Cookie。
    let (owner_id, initial_mode) = survey::Entity::find_by_id(id)
        .select_only()
        .column(survey::Column::OwnerId)
        .column(survey::Column::Mode)
        .into_tuple::<(Uuid, Option<String>)>()
        .one(&state.db)
        .await?
        .ok_or_else(AppError::not_found)?;
    let respondent_id = match initial_mode.as_deref() {
        Some("login_once") => Some(auth::authenticate(headers, &state.db).await?.id),
        Some("anonymous") | None => None,
        Some(_) => return Err(AppError::internal("unknown collection mode")),
    };

    let tx = state.db.begin().await?;
    // 全部写路径都使用 owner share → survey update 的锁序，与关闭、下架及禁用串行。
    // 这里只锁发布者记录；禁用状态不能阻止已经成功的合法重试。
    let owner = repository::lock_user_record(&tx, owner_id).await?;
    let survey = repository::lock_survey(&tx, id).await?;
    if survey.owner_id != owner_id || survey.mode != initial_mode {
        return Err(AppError::conflict(
            "collection_changed",
            "问卷填写设置已变化，请重新加载题面",
        ));
    }
    if let Some(subject_id) = respondent_id {
        repository::lock_user(&tx, subject_id).await?;
        // 等待锁时可能发生退出、改密或禁用；必须在同一事务内重新验证会话。
        if auth::authenticate(headers, &tx).await?.id != subject_id {
            return Err(AppError::unauthorized());
        }
    }
    Ok(LockedSurvey {
        tx,
        survey,
        respondent_id,
        owner_disabled: owner.disabled,
    })
}

fn ensure_available(locked: &LockedSurvey, now: DateTime<Utc>) -> AppResult<()> {
    if locked.owner_disabled {
        return Err(AppError::conflict(
            "publisher_disabled",
            "发布者账号暂不可用，问卷暂停接受填写",
        ));
    }
    repository::ensure_collecting(&locked.survey, now)
}

async fn ensure_not_submitted(locked: &LockedSurvey) -> AppResult<()> {
    let Some(respondent_id) = locked.respondent_id else {
        return Ok(());
    };
    let receipt = response::Entity::find()
        .select_only()
        .column(response::Column::Id)
        .filter(response::Column::SurveyId.eq(locked.survey.id))
        .filter(response::Column::RespondentId.eq(respondent_id))
        .into_tuple::<Uuid>()
        .one(&locked.tx)
        .await?;
    if receipt.is_some() {
        return Err(AppError::conflict(
            "already_submitted",
            "此账号已经提交过本问卷，跨发布版本也不能重复提交",
        ));
    }
    Ok(())
}

fn idempotency_conflict() -> AppError {
    AppError::conflict(
        "idempotency_conflict",
        "此提交标识已用于不同载荷，请使用原版本与原答案重试",
    )
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(public_survey, submit),
    components(schemas(
        PublicSurvey,
        PrivacyNotice,
        SubmitRequest,
        SubmissionReceipt,
        SubmissionStatus
    ))
)]
pub(crate) struct ApiDoc;
