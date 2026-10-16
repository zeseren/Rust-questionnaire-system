//! 管理员只拥有治理权限：账号状态、问卷下架和审计；没有他人答卷读取权限。

use crate::{
    AppState,
    auth::AuthUser,
    domain::Document,
    entities::{audit, session, survey, user, version},
    error::{AppError, AppResult, ErrorResponse},
    http::{ApiJson, ApiPath, ApiQuery, Pagination},
    repository,
    surveys::{SurveyStatus, effective_status},
};
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use sea_orm::{
    AccessMode, ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, FromQueryResult,
    IntoActiveModel, IsolationLevel, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Select,
    Set, TransactionTrait,
    sea_query::{Expr, LockType, extension::postgres::PgExpr},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

pub(crate) fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(users))
        .routes(routes!(change_user))
        .routes(routes!(surveys))
        .routes(routes!(survey_detail, change_survey))
        .routes(routes!(audits))
}

fn require_admin(user: &AuthUser) -> AppResult<()> {
    if user.role != "admin" {
        return Err(AppError::forbidden());
    }
    Ok(())
}

// 治理列表统一使用可重复读的只读快照：总数与分页数据来自同一时刻，且不取写锁。
async fn read_snapshot(state: &AppState) -> AppResult<DatabaseTransaction> {
    Ok(state
        .db
        .begin_with_config(
            Some(IsolationLevel::RepeatableRead),
            Some(AccessMode::ReadOnly),
        )
        .await?)
}

fn reason(value: &str) -> AppResult<()> {
    if value.contains('\0') {
        return Err(AppError::validation("治理原因不能包含 NUL 字符"));
    }
    if value.trim().is_empty() || value.chars().count() > 500 {
        return Err(AppError::validation(
            "治理原因须为 1..=500 个字符且不能全为空白",
        ));
    }
    Ok(())
}

#[derive(Deserialize, IntoParams)]
struct ListQuery {
    #[serde(flatten)]
    pagination: Pagination,
}

#[derive(Serialize, ToSchema)]
struct UserItem {
    id: Uuid,
    username: String,
    role: String,
    disabled: bool,
    created_at: DateTime<Utc>,
}
impl From<user::Model> for UserItem {
    fn from(value: user::Model) -> Self {
        Self {
            id: value.id,
            username: value.username,
            role: value.role,
            disabled: value.disabled,
            created_at: value.created_at,
        }
    }
}
#[derive(Serialize, ToSchema)]
struct UserPage {
    items: Vec<UserItem>,
    total: u64,
}

#[utoipa::path(get, path="/api/admin/users", params(("page"=Option<u64>,Query),("page_size"=Option<u64>,Query)), responses((status=200,body=UserPage),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse)), security(("cookieAuth"=[])))]
async fn users(
    State(state): State<AppState>,
    actor: AuthUser,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> AppResult<Json<UserPage>> {
    require_admin(&actor)?;
    let (offset, limit) = query.pagination.bounds()?;
    let tx = read_snapshot(&state).await?;
    let total = user::Entity::find().count(&tx).await?;
    let items = user::Entity::find()
        .order_by_desc(user::Column::CreatedAt)
        .order_by_desc(user::Column::Id)
        .offset(offset)
        .limit(limit)
        .all(&tx)
        .await?
        .into_iter()
        .map(UserItem::from)
        .collect();
    tx.commit().await?;
    Ok(Json(UserPage { items, total }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct ChangeUser {
    disabled: bool,
    reason: String,
}

#[utoipa::path(patch, path="/api/admin/users/{id}", params(("id"=Uuid,Path)), request_body=ChangeUser, responses((status=204,description="状态和审计已原子保存"),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=422,body=ErrorResponse)), security(("cookieAuth"=[])))]
async fn change_user(
    State(state): State<AppState>,
    actor: AuthUser,
    headers: axum::http::HeaderMap,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<ChangeUser>,
) -> AppResult<StatusCode> {
    require_admin(&actor)?;
    reason(&input.reason)?;
    let tx = state.db.begin().await?;
    // 与提交/登录持有的用户锁排队；禁用成功之后的新请求不能绕过状态检查。
    let target = user::Entity::find_by_id(id)
        .lock(LockType::NoKeyUpdate)
        .one(&tx)
        .await?
        .ok_or_else(AppError::not_found)?;
    // 请求体可能迟到：锁定目标后重新判定当前 Session，不能沿用提取器中的旧身份。
    // NO KEY UPDATE 不阻塞审计外键的 KEY SHARE，避免管理员交叉治理时互锁。
    // 角色不可通过治理接口更改，入口已校验管理员身份，重验后无需重复判定。
    let actor = crate::auth::authenticate(&headers, &tx).await?;
    let mut update = target.into_active_model();
    update.disabled = Set(input.disabled);
    update.update(&tx).await?;
    if input.disabled {
        session::Entity::delete_many()
            .filter(session::Column::UserId.eq(id))
            .exec(&tx)
            .await?;
    }
    audit::ActiveModel {
        id: Set(Uuid::new_v4()),
        actor_id: Set(actor.id),
        action: Set(if input.disabled {
            "disable_user"
        } else {
            "enable_user"
        }
        .to_owned()),
        target_id: Set(id),
        reason: Set(input.reason),
        created_at: Set(Utc::now()),
    }
    .insert(&tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// 列表与详情只从 JSONB 中取标题，不把整份 Document 加载到内存。
#[derive(FromQueryResult)]
struct SurveyRow {
    id: Uuid,
    owner_id: Uuid,
    title: String,
    status: String,
    taken_down: bool,
    deadline: Option<DateTime<Utc>>,
    current_version_id: Option<Uuid>,
}

#[derive(Serialize, ToSchema)]
struct SurveyItem {
    id: Uuid,
    owner_id: Uuid,
    title: String,
    status: SurveyStatus,
    taken_down: bool,
    deadline: Option<DateTime<Utc>>,
    current_version_id: Option<Uuid>,
}
impl SurveyItem {
    fn from_row(row: SurveyRow) -> AppResult<Self> {
        Ok(Self {
            status: effective_status(&row.status, row.deadline, Utc::now())?,
            id: row.id,
            owner_id: row.owner_id,
            title: row.title,
            taken_down: row.taken_down,
            deadline: row.deadline,
            current_version_id: row.current_version_id,
        })
    }
}

// 治理列表与详情共用的投影：只取治理列与草稿标题。
fn governance_query() -> Select<survey::Entity> {
    survey::Entity::find()
        .select_only()
        .columns([
            survey::Column::Id,
            survey::Column::OwnerId,
            survey::Column::Status,
            survey::Column::TakenDown,
            survey::Column::Deadline,
            survey::Column::CurrentVersionId,
        ])
        .column_as(
            Expr::col(survey::Column::Draft).cast_json_field("title"),
            "title",
        )
}
#[derive(Serialize, ToSchema)]
struct AdminSurveyPage {
    items: Vec<SurveyItem>,
    total: u64,
}
#[derive(Serialize, ToSchema)]
struct GovernanceDetail {
    survey: SurveyItem,
    published_document: Option<Document>,
}

#[utoipa::path(get, path="/api/admin/surveys", params(("page"=Option<u64>,Query),("page_size"=Option<u64>,Query)), responses((status=200,body=AdminSurveyPage),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse)), security(("cookieAuth"=[])))]
async fn surveys(
    State(state): State<AppState>,
    actor: AuthUser,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> AppResult<Json<AdminSurveyPage>> {
    require_admin(&actor)?;
    let (offset, limit) = query.pagination.bounds()?;
    // 只读快照：总数与分页数据来自同一时刻，且不为列表取任何写锁。
    let tx = read_snapshot(&state).await?;
    let total = survey::Entity::find().count(&tx).await?;
    let items = governance_query()
        .order_by_desc(survey::Column::CreatedAt)
        .order_by_desc(survey::Column::Id)
        .offset(offset)
        .limit(limit)
        .into_model::<SurveyRow>()
        .all(&tx)
        .await?
        .into_iter()
        .map(SurveyItem::from_row)
        .collect::<AppResult<_>>()?;
    tx.commit().await?;
    Ok(Json(AdminSurveyPage { items, total }))
}

#[utoipa::path(get, path="/api/admin/surveys/{id}", params(("id"=Uuid,Path)), responses((status=200,body=GovernanceDetail),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse)), security(("cookieAuth"=[])))]
async fn survey_detail(
    State(state): State<AppState>,
    actor: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
) -> AppResult<Json<GovernanceDetail>> {
    require_admin(&actor)?;
    let row = governance_query()
        .filter(survey::Column::Id.eq(id))
        .into_model::<SurveyRow>()
        .one(&state.db)
        .await?
        .ok_or_else(AppError::not_found)?;
    let published_document = if let Some(version_id) = row.current_version_id {
        let version = version::Entity::find_by_id(version_id)
            .filter(version::Column::SurveyId.eq(id))
            .one(&state.db)
            .await?
            .ok_or_else(AppError::not_found)?;
        Some(serde_json::from_value(version.document)?)
    } else {
        None
    };
    Ok(Json(GovernanceDetail {
        survey: SurveyItem::from_row(row)?,
        published_document,
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct ChangeSurvey {
    taken_down: bool,
    reason: String,
}

#[utoipa::path(patch, path="/api/admin/surveys/{id}", params(("id"=Uuid,Path)), request_body=ChangeSurvey, responses((status=204,description="治理标志和审计已原子保存；不改变回收状态"),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=422,body=ErrorResponse)), security(("cookieAuth"=[])))]
async fn change_survey(
    State(state): State<AppState>,
    actor: AuthUser,
    headers: axum::http::HeaderMap,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<ChangeSurvey>,
) -> AppResult<StatusCode> {
    require_admin(&actor)?;
    reason(&input.reason)?;
    let target = survey::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(AppError::not_found)?;
    let tx = state.db.begin().await?;
    repository::lock_user_record(&tx, target.owner_id).await?;
    let target = repository::lock_survey(&tx, id).await?;
    // 同上：只重验 Session，管理员身份已在入口校验且角色不可变。
    let actor = crate::auth::authenticate(&headers, &tx).await?;
    let mut update = target.into_active_model();
    update.taken_down = Set(input.taken_down);
    update.updated_at = Set(Utc::now());
    update.update(&tx).await?;
    audit::ActiveModel {
        id: Set(Uuid::new_v4()),
        actor_id: Set(actor.id),
        action: Set(if input.taken_down {
            "take_down"
        } else {
            "restore_survey"
        }
        .to_owned()),
        target_id: Set(id),
        reason: Set(input.reason),
        created_at: Set(Utc::now()),
    }
    .insert(&tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
struct AuditItem {
    id: Uuid,
    actor_id: Uuid,
    action: String,
    target_id: Uuid,
    reason: String,
    created_at: DateTime<Utc>,
}
#[derive(Serialize, ToSchema)]
struct AuditPage {
    items: Vec<AuditItem>,
    total: u64,
}

#[utoipa::path(get, path="/api/admin/audits", params(("page"=Option<u64>,Query),("page_size"=Option<u64>,Query)), responses((status=200,body=AuditPage),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse)), security(("cookieAuth"=[])))]
async fn audits(
    State(state): State<AppState>,
    actor: AuthUser,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> AppResult<Json<AuditPage>> {
    require_admin(&actor)?;
    let (offset, limit) = query.pagination.bounds()?;
    let tx = read_snapshot(&state).await?;
    let total = audit::Entity::find().count(&tx).await?;
    let items = audit::Entity::find()
        .order_by_desc(audit::Column::CreatedAt)
        .order_by_desc(audit::Column::Id)
        .offset(offset)
        .limit(limit)
        .all(&tx)
        .await?
        .into_iter()
        .map(|item| AuditItem {
            id: item.id,
            actor_id: item.actor_id,
            action: item.action,
            target_id: item.target_id,
            reason: item.reason,
            created_at: item.created_at,
        })
        .collect();
    tx.commit().await?;
    Ok(Json(AuditPage { items, total }))
}
