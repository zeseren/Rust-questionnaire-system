//! 共享的归属查询和行锁。所有回收写操作先锁用户，再锁问卷，形成一致的并发顺序。

use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QuerySelect, Set, sea_query::LockType,
};
use uuid::Uuid;

use crate::{
    auth,
    entities::{survey, user},
    error::{AppError, AppResult},
    surveys::SurveyStatus,
};

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

pub(crate) async fn expire_survey(
    tx: &DatabaseTransaction,
    survey: survey::Model,
    now: DateTime<Utc>,
) -> AppResult<survey::Model> {
    if survey.status == "collecting" && survey.deadline.is_some_and(|deadline| deadline <= now) {
        let mut update = survey.into_active_model();
        update.status = Set("closed".to_owned());
        update.updated_at = Set(now);
        return Ok(update.update(tx).await?);
    }
    Ok(survey)
}

pub(crate) fn ensure_collecting(survey: &survey::Model, now: DateTime<Utc>) -> AppResult<()> {
    if survey.taken_down {
        return Err(AppError::conflict("taken_down", "问卷已被下架"));
    }
    if survey.deadline.is_some_and(|deadline| deadline <= now) {
        return Err(AppError::conflict("deadline_passed", "问卷已到截止时间"));
    }
    match SurveyStatus::from_stored(&survey.status)? {
        SurveyStatus::Collecting => Ok(()),
        SurveyStatus::Unpublished => Err(AppError::not_found()),
        SurveyStatus::Closed => Err(AppError::conflict("collection_closed", "问卷已关闭回收")),
        SurveyStatus::Archived => Err(AppError::conflict("archived", "问卷已归档")),
    }
}
