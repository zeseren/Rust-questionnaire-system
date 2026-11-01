//! 公开填写入口：题面只读与新答卷共用回收资格检查，成功重试不重新占用填写次数。

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect, Set,
    TransactionTrait, sea_query::LockType,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::{Uuid, Variant, Version};

use crate::{
    AppState, auth,
    domain::{Answer, Document},
    entities::{FillingMode, response, survey, user},
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
    pub mode: FillingMode,
    pub deadline: Option<DateTime<Utc>>,
    /// purpose 是发布者说明的收集目的；文本只能作为普通文本展示。
    #[schema(value_type = Document)]
    pub document: Arc<Document>,
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
    survey: repository::SurveyMetaRow,
    respondent_id: Option<Uuid>,
    owner_disabled: bool,
}

#[derive(Serialize)]
struct NormalizedPayload<'a> {
    version_id: Uuid,
    answers: &'a BTreeMap<Uuid, Value>,
}

pub(crate) fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(public_survey))
        .routes(routes!(submit))
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

/// 题面读取不需要写锁，也不开启只读事务：元数据、题面与是否已提交各自直接查询。
async fn read_public_survey(
    state: &AppState,
    headers: &HeaderMap,
    id: Uuid,
) -> AppResult<PublicSurvey> {
    // 元数据投影不包含 1 MiB 草稿；题面来自不可变的发布版本。
    let survey = survey::Entity::find_by_id(id)
        .select_only()
        .columns(repository::survey_meta_columns())
        .into_model::<repository::SurveyMetaRow>()
        .one(&state.db)
        .await?
        .ok_or_else(AppError::not_found)?;
    // anonymous 与尚未设置的模式完全不解释 Cookie；只有 login_once 才要求有效会话。
    let (mode, respondent_id) = match survey.mode.unwrap_or(FillingMode::Anonymous) {
        FillingMode::LoginOnce => (
            FillingMode::LoginOnce,
            Some(auth::authenticate(headers, &state.db).await?.id),
        ),
        FillingMode::Anonymous => (FillingMode::Anonymous, None),
    };
    let publisher_disabled = user::Entity::find_by_id(survey.owner_id)
        .select_only()
        .column(user::Column::Disabled)
        .into_tuple::<bool>()
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::internal("survey publisher is missing"))?;
    if publisher_disabled {
        return Err(AppError::conflict(
            "publisher_disabled",
            "发布者账号暂不可用，问卷暂停接受填写",
        ));
    }
    repository::ensure_collecting(
        survey.status,
        survey.deadline,
        survey.taken_down,
        Utc::now(),
    )?;
    if let Some(respondent_id) = respondent_id {
        ensure_not_submitted(&state.db, id, respondent_id).await?;
    }
    let version_id = survey
        .current_version_id
        .ok_or_else(|| AppError::internal("collecting survey has no version"))?;
    let version = repository::load_version(&state.db, id, version_id)
        .await?
        .ok_or_else(|| AppError::internal("current version is missing"))?;
    Ok(PublicSurvey {
        survey_id: id,
        version_id,
        version_number: version.number,
        mode,
        deadline: survey.deadline,
        document: version.prepared.document_arc(),
        privacy: privacy_notice(mode),
    })
}

async fn submit_response(
    state: &AppState,
    headers: &HeaderMap,
    id: Uuid,
    request: SubmitRequest,
) -> AppResult<(StatusCode, SubmissionReceipt)> {
    validate_idempotency_key(request.idempotency_key)?;
    // 只保存秘密的 SHA-256；UUID 的文本大小写、连字符格式不会改变其身份。
    let key_hash = auth::hash_secret_bytes(request.idempotency_key.as_bytes());
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
        if let Some(respondent_id) = locked.respondent_id {
            ensure_not_submitted(&locked.tx, id, respondent_id).await?;
        }
    }
    let published = repository::load_version(&locked.tx, id, request.version_id)
        .await?
        .ok_or_else(|| AppError::validation("version_id 不属于本问卷的已发布版本"))?;
    if previous.is_none() && locked.survey.current_version_id != Some(request.version_id) {
        return Err(AppError::conflict(
            "version_changed",
            "问卷已更新，请保留当前输入并重新加载题面核对",
        ));
    }

    // 历史版本不可变：旧版本重试仍按旧规则规范化；有逻辑时从入口重建路径，
    // 只有已校验且适用的答案进入下游条件，客户端不能指定可信历史或可见题目。
    // 加载版本时已编译规则与流程，提交校验直接复用同一份编译结果。
    let answers = published.prepared.validate_answers(request.answers)?;
    let payload_hash = submission_payload_hash(request.version_id, &answers)?;
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
    let answers_json = serde_json::Value::Object(
        answers
            .into_iter()
            .map(|(question_id, value)| (question_id.to_string(), value))
            .collect(),
    );
    response::Entity::insert(response::ActiveModel {
        id: Set(receipt_id),
        survey_id: Set(id),
        version_id: Set(request.version_id),
        respondent_id: Set(locked.respondent_id),
        scope: Set(scope),
        key_hash: Set(key_hash),
        payload_hash: Set(payload_hash),
        answers: Set(answers_json),
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
    let tx = state.db.begin().await?;
    // 共享锁读取元数据行：与关闭/下架等写操作互斥，并发提交之间不互相阻塞。
    // 提交路径只读问卷行；到期由 `ensure_collecting` 按截止时间推导，不在此物化。
    let survey = survey::Entity::find_by_id(id)
        .select_only()
        .columns(repository::survey_meta_columns())
        .lock(LockType::Share)
        .into_model::<repository::SurveyMetaRow>()
        .one(&tx)
        .await?
        .ok_or_else(AppError::not_found)?;
    // anonymous 与尚未设置的模式绝不解释 Cookie；login_once 只在这里认证一次。
    let respondent_id = match survey.mode {
        Some(FillingMode::LoginOnce) => Some(auth::authenticate(headers, &tx).await?.id),
        Some(FillingMode::Anonymous) | None => None,
    };
    // 发布者禁用状态在同一事务内读取；禁用不能阻止已经成功的合法重试。
    let owner_disabled = user::Entity::find_by_id(survey.owner_id)
        .select_only()
        .column(user::Column::Disabled)
        .into_tuple::<bool>()
        .one(&tx)
        .await?
        .ok_or_else(|| AppError::internal("survey publisher is missing"))?;
    Ok(LockedSurvey {
        tx,
        survey,
        respondent_id,
        owner_disabled,
    })
}

fn ensure_available(locked: &LockedSurvey, now: DateTime<Utc>) -> AppResult<()> {
    if locked.owner_disabled {
        return Err(AppError::conflict(
            "publisher_disabled",
            "发布者账号暂不可用，问卷暂停接受填写",
        ));
    }
    repository::ensure_collecting(
        locked.survey.status,
        locked.survey.deadline,
        locked.survey.taken_down,
        now,
    )
}

async fn ensure_not_submitted(
    db: &impl ConnectionTrait,
    survey_id: Uuid,
    respondent_id: Uuid,
) -> AppResult<()> {
    let receipt = response::Entity::find()
        .select_only()
        .column(response::Column::Id)
        .filter(response::Column::SurveyId.eq(survey_id))
        .filter(response::Column::RespondentId.eq(respondent_id))
        .into_tuple::<Uuid>()
        .one(db)
        .await?;
    if receipt.is_some() {
        return Err(AppError::conflict(
            "already_submitted",
            "此账号已经提交过本问卷，跨发布版本也不能重复提交",
        ));
    }
    Ok(())
}

/// 幂等秘密必须是浏览器安全随机生成的 UUIDv4（RFC 4122 变体）。
///
/// 随机版本与变体是「同一逻辑提交只生成一次」的前提；顺序 UUID 或自定义变体
/// 会让不同客户端猜中同一作用域记录。
pub(crate) fn validate_idempotency_key(key: Uuid) -> AppResult<()> {
    if key.get_version() != Some(Version::Random) || key.get_variant() != Variant::RFC4122 {
        return Err(AppError::validation(
            "idempotency_key 必须是安全随机生成的 UUIDv4",
        ));
    }
    Ok(())
}

/// 把 `std::io::Write` 直接接到 SHA-256：`digest` 0.11 不再提供该实现，
/// 这里用零分配的适配器保持与 `serde_json::to_writer` 的流式写入。
struct HashWriter(Sha256);

impl std::io::Write for HashWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.write_all(buffer)?;
        Ok(buffer.len())
    }

    fn write_all(&mut self, buffer: &[u8]) -> std::io::Result<()> {
        self.0.update(buffer);
        Ok(())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 规范化载荷的稳定摘要：同一版本与同一规范化答案必然得到同一哈希。
///
/// 直接流式写入 SHA-256，避免先把整份载荷序列化成中间字节缓冲。
pub(crate) fn submission_payload_hash(
    version_id: Uuid,
    answers: &BTreeMap<Uuid, Value>,
) -> AppResult<String> {
    let payload = NormalizedPayload {
        version_id,
        answers,
    };
    let mut hasher = HashWriter(Sha256::new());
    serde_json::to_writer(&mut hasher, &payload)?;
    Ok(hex::encode(hasher.0.finalize()))
}

/// 填写前的知情提示：只有登录限次模式保留账号关联用于限次，不能宣称为完全匿名。
pub(crate) fn privacy_notice(mode: FillingMode) -> PrivacyNotice {
    let mode_notice = match mode {
        FillingMode::Anonymous => {
            "公开匿名：即使已登录，答卷也不关联账号。不承诺按自然人限次，也不宣称不可追踪或严格一人一票。"
        }
        FillingMode::LoginOnce => {
            "登录限次：同一账号对本问卷跨版本最多提交一次。平台保留内部账号关联用于限次，不能称为完全匿名；发布者页面及导出不提供账号信息。"
        }
    };
    PrivacyNotice {
        mode_notice,
        data_visibility: "答卷、统计和导出仅向本问卷发布者开放，不默认公开结果；管理员治理接口不提供他人答卷。数据库运维权限与业务角色不同。",
        sensitive_data_notice: "请阅读调查目的，仅提交调查所需的非敏感信息，不要填写密码、证件号码等不必要的敏感信息。当前系统用于教学演示及知情同意的非敏感调查。",
        public_link_notice: "公开链接不是保密访问控制，获得链接的人可能访问题面。系统不采集设备指纹或地理位置进入答卷，匿名不意味着数据库运维人员无法接触数据。",
    }
}

pub(crate) fn idempotency_conflict() -> AppError {
    AppError::conflict(
        "idempotency_conflict",
        "此提交标识已用于不同载荷，请使用原版本与原答案重试",
    )
}
