//! 共享的归属查询和行锁。所有回收写操作先锁用户，再锁问卷，形成一致的并发顺序。

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QuerySelect, Set, sea_query::LockType,
};
use uuid::Uuid;

use crate::{
    entities::{survey, user},
    error::{AppError, AppResult},
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

pub(crate) async fn lock_owned_survey(
    tx: &DatabaseTransaction,
    owner_id: Uuid,
    id: Uuid,
) -> AppResult<survey::Model> {
    lock_user(tx, owner_id).await?;
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
    match survey.status.as_str() {
        "collecting" => Ok(()),
        "unpublished" => Err(AppError::not_found()),
        "closed" => Err(AppError::conflict("collection_closed", "问卷已关闭回收")),
        "archived" => Err(AppError::conflict("archived", "问卷已归档")),
        _ => Err(AppError::internal("invalid persisted survey state")),
    }
}
