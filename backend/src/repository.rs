//! 共享的归属查询和行锁。所有回收写操作先锁用户，再锁问卷，形成一致的并发顺序。
//!
//! 只读路径共用 `read_snapshot`，列表查询共用 `title_column` 与 `owned_survey_meta`，
//! 避免各模块重复配置隔离级别或把整份 JsonBinary 草稿搬进内存。

use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use sea_orm::{
    AccessMode, ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection,
    DatabaseTransaction, EntityTrait, FromQueryResult, IntoActiveModel, IsolationLevel,
    QueryFilter, QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, LockType, SimpleExpr, extension::postgres::PgExpr},
};
use uuid::Uuid;

use crate::{
    auth,
    entities::{SurveyStatus, survey, user},
    error::{AppError, AppResult},
};

/// 可重复读的只读事务：同一响应内的总数与分页数据来自同一快照，且不取写锁。
pub(crate) async fn read_snapshot(db: &DatabaseConnection) -> AppResult<DatabaseTransaction> {
    Ok(db
        .begin_with_config(
            Some(IsolationLevel::RepeatableRead),
            Some(AccessMode::ReadOnly),
        )
        .await?)
}

/// 只从草稿 JSONB 中取出标题列，供各列表投影复用。
pub(crate) fn title_column() -> SimpleExpr {
    Expr::col(survey::Column::Draft).cast_json_field("title")
}

/// 归属校验只需要 ID 与当前版本，不加载 1 MiB 的草稿。
#[derive(FromQueryResult)]
pub(crate) struct SurveyMeta {
    pub id: Uuid,
    pub current_version_id: Option<Uuid>,
}

pub(crate) async fn owned_survey_meta(
    db: &impl ConnectionTrait,
    owner_id: Uuid,
    id: Uuid,
) -> AppResult<SurveyMeta> {
    survey::Entity::find_by_id(id)
        .filter(survey::Column::OwnerId.eq(owner_id))
        .select_only()
        .columns([survey::Column::Id, survey::Column::CurrentVersionId])
        .into_model::<SurveyMeta>()
        .one(db)
        .await?
        .ok_or_else(AppError::not_found)
}

pub(crate) async fn lock_user_record(tx: &DatabaseTransaction, id: Uuid) -> AppResult<user::Model> {
    user::Entity::find_by_id(id)
        .lock(LockType::Share)
        .one(tx)
        .await?
        .ok_or_else(AppError::not_found)
}

pub(crate) async fn lock_user(tx: &DatabaseTransaction, id: Uuid) -> AppResult<user::Model> {
    let user = lock_user_record(tx, id).await?;
    if user.disabled {
        return Err(AppError::unauthorized());
    }
    Ok(user)
}

/// 锁定当前登录账号，并在同一事务内重验请求携带的原 Session。
///
/// 提取器在读取请求体之前就完成认证；等待请求体或行锁期间可能发生退出、改密或禁用。
/// 这些操作都要取得同一账号的 NO KEY UPDATE 锁，所以持锁后的重验不会再看到过期身份。
pub(crate) async fn lock_session_user(
    tx: &DatabaseTransaction,
    headers: &HeaderMap,
    id: Uuid,
) -> AppResult<user::Model> {
    let user = lock_user(tx, id).await?;
    if auth::authenticate(headers, tx).await?.id != id {
        return Err(AppError::unauthorized());
    }
    Ok(user)
}

pub(crate) async fn lock_survey(tx: &DatabaseTransaction, id: Uuid) -> AppResult<survey::Model> {
    survey::Entity::find_by_id(id)
        .lock(LockType::Update)
        .one(tx)
        .await?
        .ok_or_else(AppError::not_found)
}

pub(crate) async fn owned_survey(
    db: &impl ConnectionTrait,
    owner_id: Uuid,
    id: Uuid,
) -> AppResult<survey::Model> {
    survey::Entity::find_by_id(id)
        .filter(survey::Column::OwnerId.eq(owner_id))
        .one(db)
        .await?
        .ok_or_else(AppError::not_found)
}

/// 所有者写入的统一入口：先锁并重验当前账号，再锁问卷并核对归属。
pub(crate) async fn lock_owned_survey(
    tx: &DatabaseTransaction,
    headers: &HeaderMap,
    owner_id: Uuid,
    id: Uuid,
) -> AppResult<survey::Model> {
    lock_session_user(tx, headers, owner_id).await?;
    let survey = lock_survey(tx, id).await?;
    if survey.owner_id != owner_id {
        return Err(AppError::not_found());
    }
    Ok(survey)
}

/// 把“回收中但已过截止时间”物化为已关闭；状态未变化时不产生写入。
pub(crate) async fn expire_survey(
    tx: &DatabaseTransaction,
    survey: survey::Model,
    now: DateTime<Utc>,
) -> AppResult<survey::Model> {
    if survey.status.effective(survey.deadline, now) == survey.status {
        return Ok(survey);
    }
    let mut update = survey.into_active_model();
    update.status = Set(SurveyStatus::Closed);
    update.updated_at = Set(now);
    Ok(update.update(tx).await?)
}

/// 新提交的资格检查；成功重试不调用本函数。
pub(crate) fn ensure_collecting(
    status: SurveyStatus,
    deadline: Option<DateTime<Utc>>,
    taken_down: bool,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if taken_down {
        return Err(AppError::conflict("taken_down", "问卷已被下架"));
    }
    match status.effective(deadline, now) {
        SurveyStatus::Collecting => Ok(()),
        SurveyStatus::Unpublished => Err(AppError::not_found()),
        SurveyStatus::Closed if status == SurveyStatus::Collecting => {
            Err(AppError::conflict("deadline_passed", "问卷已到截止时间"))
        }
        SurveyStatus::Closed => Err(AppError::conflict("collection_closed", "问卷已关闭回收")),
        SurveyStatus::Archived => Err(AppError::conflict("archived", "问卷已归档")),
    }
}
