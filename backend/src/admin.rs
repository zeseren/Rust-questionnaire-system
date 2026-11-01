//! 管理员只拥有治理权限：账号状态、问卷下架和审计；没有他人答卷读取权限。

use crate::{
    AppState,
    auth::AuthUser,
    domain::Document,
    entities::{AuditAction, SurveyStatus, UserRole, audit, session, survey, user, version},
    error::{AppError, AppResult, ErrorResponse},
    http::{ApiJson, ApiPath, ApiQuery, Page, Pagination},
    repository,
};
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, FromQueryResult, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Select, Set, TransactionTrait, sea_query::LockType,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
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

pub(crate) fn require_admin(user: &AuthUser) -> AppResult<()> {
    if user.role != UserRole::Admin {
        return Err(AppError::forbidden());
    }
    Ok(())
}

pub(crate) fn reason(value: &str) -> AppResult<()> {
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

#[derive(Serialize, FromQueryResult, ToSchema)]
struct UserItem {
    id: Uuid,
    username: String,
    role: UserRole,
    disabled: bool,
    created_at: DateTime<Utc>,
}

#[utoipa::path(get, path="/api/admin/users", params(Pagination), responses((status=200,body=Page<UserItem>),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse)), security(("cookieAuth"=[])))]
async fn users(
    State(state): State<AppState>,
    actor: AuthUser,
    ApiQuery(pagination): ApiQuery<Pagination>,
) -> AppResult<Json<Page<UserItem>>> {
    require_admin(&actor)?;
    let bounds = pagination.bounds()?;
    let total = user::Entity::find().count(&state.db).await?;
    let items = user::Entity::find()
        .select_only()
        .columns([
            user::Column::Id,
            user::Column::Username,
            user::Column::Role,
            user::Column::Disabled,
            user::Column::CreatedAt,
        ])
        .order_by_desc(user::Column::CreatedAt)
        .order_by_desc(user::Column::Id)
        .offset(bounds.offset)
        .limit(bounds.limit)
        .into_model::<UserItem>()
        .all(&state.db)
        .await?;
    Ok(Json(Page::new(items, total, bounds)))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangeUser {
    pub(crate) disabled: bool,
    pub(crate) reason: String,
}

#[utoipa::path(patch, path="/api/admin/users/{id}", params(("id"=Uuid,Path)), request_body=ChangeUser, responses((status=204,description="状态和审计已原子保存"),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=422,body=ErrorResponse)), security(("cookieAuth"=[])))]
async fn change_user(
    State(state): State<AppState>,
    actor: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<ChangeUser>,
) -> AppResult<StatusCode> {
    require_admin(&actor)?;
    reason(&input.reason)?;
    let tx = state.db.begin().await?;
    // 锁定目标账号：禁用成功之后的新请求不能绕过状态检查，审计可原子提交。
    // NO KEY UPDATE 不阻塞审计外键的 KEY SHARE，避免管理员交叉治理时互锁。
    let target = user::Entity::find_by_id(id)
        .lock(LockType::NoKeyUpdate)
        .one(&tx)
        .await?
        .ok_or_else(AppError::not_found)?;
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
            AuditAction::DisableUser
        } else {
            AuditAction::EnableUser
        }),
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
pub(crate) struct SurveyRow {
    pub(crate) id: Uuid,
    pub(crate) owner_id: Uuid,
    pub(crate) title: String,
    pub(crate) status: SurveyStatus,
    pub(crate) taken_down: bool,
    pub(crate) deadline: Option<DateTime<Utc>>,
    pub(crate) current_version_id: Option<Uuid>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct SurveyItem {
    pub(crate) id: Uuid,
    pub(crate) owner_id: Uuid,
    pub(crate) title: String,
    pub(crate) status: SurveyStatus,
    pub(crate) taken_down: bool,
    pub(crate) deadline: Option<DateTime<Utc>>,
    pub(crate) current_version_id: Option<Uuid>,
}
impl SurveyItem {
    pub(crate) fn from_row(row: SurveyRow, now: DateTime<Utc>) -> Self {
        Self {
            status: row.status.effective(row.deadline, now),
            id: row.id,
            owner_id: row.owner_id,
            title: row.title,
            taken_down: row.taken_down,
            deadline: row.deadline,
            current_version_id: row.current_version_id,
        }
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
        .column_as(repository::title_column(), "title")
}
#[derive(Serialize, ToSchema)]
struct GovernanceDetail {
    survey: SurveyItem,
    published_document: Option<Document>,
}

#[utoipa::path(get, path="/api/admin/surveys", params(Pagination), responses((status=200,body=Page<SurveyItem>),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse)), security(("cookieAuth"=[])))]
async fn surveys(
    State(state): State<AppState>,
    actor: AuthUser,
    ApiQuery(pagination): ApiQuery<Pagination>,
) -> AppResult<Json<Page<SurveyItem>>> {
    require_admin(&actor)?;
    let bounds = pagination.bounds()?;
    let now = Utc::now();
    // 列表只做只读查询，不为分页读取任何写锁。
    let total = survey::Entity::find().count(&state.db).await?;
    let items = governance_query()
        .order_by_desc(survey::Column::CreatedAt)
        .order_by_desc(survey::Column::Id)
        .offset(bounds.offset)
        .limit(bounds.limit)
        .into_model::<SurveyRow>()
        .all(&state.db)
        .await?
        .into_iter()
        .map(|row| SurveyItem::from_row(row, now))
        .collect();
    Ok(Json(Page::new(items, total, bounds)))
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
        survey: SurveyItem::from_row(row, Utc::now()),
        published_document,
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangeSurvey {
    pub(crate) taken_down: bool,
    pub(crate) reason: String,
}

#[utoipa::path(patch, path="/api/admin/surveys/{id}", params(("id"=Uuid,Path)), request_body=ChangeSurvey, responses((status=204,description="治理标志和审计已原子保存；不改变回收状态"),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse),(status=404,body=ErrorResponse),(status=422,body=ErrorResponse)), security(("cookieAuth"=[])))]
async fn change_survey(
    State(state): State<AppState>,
    actor: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<ChangeSurvey>,
) -> AppResult<StatusCode> {
    require_admin(&actor)?;
    reason(&input.reason)?;
    let tx = state.db.begin().await?;
    // 直接锁定目标问卷元数据行：归属不再需要预读，下架与回收写入仍按同一行串行。
    repository::lock_survey_meta(&tx, id).await?;
    let now = repository::now_micros();
    survey::ActiveModel {
        id: Set(id),
        taken_down: Set(input.taken_down),
        updated_at: Set(now),
        ..Default::default()
    }
    .update_without_returning(&tx)
    .await?;
    audit::ActiveModel {
        id: Set(Uuid::new_v4()),
        actor_id: Set(actor.id),
        action: Set(if input.taken_down {
            AuditAction::TakedownSurvey
        } else {
            AuditAction::RestoreSurvey
        }),
        target_id: Set(id),
        reason: Set(input.reason),
        created_at: Set(now),
    }
    .insert(&tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, FromQueryResult, ToSchema)]
struct AuditItem {
    id: Uuid,
    actor_id: Uuid,
    action: AuditAction,
    target_id: Uuid,
    reason: String,
    created_at: DateTime<Utc>,
}

#[utoipa::path(get, path="/api/admin/audits", params(Pagination), responses((status=200,body=Page<AuditItem>),(status=401,body=ErrorResponse),(status=403,body=ErrorResponse)), security(("cookieAuth"=[])))]
async fn audits(
    State(state): State<AppState>,
    actor: AuthUser,
    ApiQuery(pagination): ApiQuery<Pagination>,
) -> AppResult<Json<Page<AuditItem>>> {
    require_admin(&actor)?;
    let bounds = pagination.bounds()?;
    let total = audit::Entity::find().count(&state.db).await?;
    let items = audit::Entity::find()
        .order_by_desc(audit::Column::CreatedAt)
        .order_by_desc(audit::Column::Id)
        .offset(bounds.offset)
        .limit(bounds.limit)
        .into_model::<AuditItem>()
        .all(&state.db)
        .await?;
    Ok(Json(Page::new(items, total, bounds)))
}
