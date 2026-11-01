//! 共享的归属查询和行锁。涉及问卷的写操作先锁问卷元数据行，再在同一事务内完成写入。
//!
//! 列表查询共用 `title_column` 与 `owned_survey_meta`，避免把整份 JsonBinary 草稿搬进内存；
//! 已发布版本不可变，`load_version` 统一加载并编译发布版本题面。

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    FromQueryResult, QueryFilter, QuerySelect, Set,
    sea_query::{Expr, LockType, SimpleExpr, extension::postgres::PgExpr},
};
use uuid::Uuid;

use crate::{
    domain::PreparedDocument,
    entities::{FillingMode, SurveyStatus, survey, version},
    error::{AppError, AppResult},
};

/// 只从草稿 JSONB 中取出标题列，供各列表投影复用。
pub(crate) fn title_column() -> SimpleExpr {
    Expr::col(survey::Column::Draft).cast_json_field("title")
}

/// 与 PostgreSQL `timestamptz` 一致的微秒时间：写响应与后续读回值不再有亚微秒差异。
pub(crate) fn now_micros() -> DateTime<Utc> {
    DateTime::from_timestamp_micros(Utc::now().timestamp_micros())
        .expect("当前系统时间在合法微秒范围内")
}

/// 问卷元数据投影：包含行锁与过期物化所需的全部列，但不含 1 MiB 的 `draft`。
#[derive(FromQueryResult)]
pub(crate) struct SurveyMetaRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub status: SurveyStatus,
    pub mode: Option<FillingMode>,
    pub deadline: Option<DateTime<Utc>>,
    pub taken_down: bool,
    pub revision: i64,
    pub current_version_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&survey::Model> for SurveyMetaRow {
    fn from(record: &survey::Model) -> Self {
        Self {
            id: record.id,
            owner_id: record.owner_id,
            status: record.status,
            mode: record.mode,
            deadline: record.deadline,
            taken_down: record.taken_down,
            revision: record.revision,
            current_version_id: record.current_version_id,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

/// 元数据投影的列清单，供只读题面与行锁共用，避免两处字段顺序漂移。
pub(crate) fn survey_meta_columns() -> [survey::Column; 10] {
    [
        survey::Column::Id,
        survey::Column::OwnerId,
        survey::Column::Status,
        survey::Column::Mode,
        survey::Column::Deadline,
        survey::Column::TakenDown,
        survey::Column::Revision,
        survey::Column::CurrentVersionId,
        survey::Column::CreatedAt,
        survey::Column::UpdatedAt,
    ]
}

/// 按版本行编译出的发布版本；每次请求独立加载，不跨请求共享。
pub(crate) struct PublishedVersion {
    pub id: Uuid,
    pub number: i32,
    pub prepared: PreparedDocument,
}

/// 只投影编译所需的版本列；不搬运整行元数据。
#[derive(FromQueryResult)]
struct VersionRow {
    id: Uuid,
    number: i32,
    document: serde_json::Value,
}

/// 按问卷归属加载并编译发布版本；不存在或不属于该问卷时返回 `None`。
pub(crate) async fn load_version(
    db: &impl ConnectionTrait,
    survey_id: Uuid,
    version_id: Uuid,
) -> AppResult<Option<PublishedVersion>> {
    let Some(row) = version::Entity::find_by_id(version_id)
        .filter(version::Column::SurveyId.eq(survey_id))
        .select_only()
        .columns([
            version::Column::Id,
            version::Column::Number,
            version::Column::Document,
        ])
        .into_model::<VersionRow>()
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(PublishedVersion {
        id: row.id,
        number: row.number,
        prepared: PreparedDocument::new(serde_json::from_value(row.document)?)?,
    }))
}

/// 只读取已保存草稿的单列投影；响应构造不再搬回整行元数据。
pub(crate) async fn load_survey_draft(
    db: &impl ConnectionTrait,
    id: Uuid,
) -> AppResult<serde_json::Value> {
    survey::Entity::find_by_id(id)
        .select_only()
        .column(survey::Column::Draft)
        .into_tuple::<serde_json::Value>()
        .one(db)
        .await?
        .ok_or_else(AppError::not_found)
}

/// 锁问卷元数据行，不读取草稿；锁序仍与所有写路径一致。
pub(crate) async fn lock_survey_meta(
    tx: &DatabaseTransaction,
    id: Uuid,
) -> AppResult<SurveyMetaRow> {
    survey::Entity::find_by_id(id)
        .select_only()
        .columns(survey_meta_columns())
        .lock(LockType::Update)
        .into_model::<SurveyMetaRow>()
        .one(tx)
        .await?
        .ok_or_else(AppError::not_found)
}

/// 所有者写入的元数据入口：锁元数据行并核对归属；不匹配按不存在处理。
pub(crate) async fn lock_owned_survey_meta(
    tx: &DatabaseTransaction,
    owner_id: Uuid,
    id: Uuid,
) -> AppResult<SurveyMetaRow> {
    let survey = lock_survey_meta(tx, id).await?;
    if survey.owner_id != owner_id {
        return Err(AppError::not_found());
    }
    Ok(survey)
}

/// 把“回收中但已过截止时间”物化为已关闭；只写状态与更新时间，不重写草稿列。
pub(crate) async fn expire_survey(
    tx: &DatabaseTransaction,
    mut survey: SurveyMetaRow,
    now: DateTime<Utc>,
) -> AppResult<SurveyMetaRow> {
    if survey.status.effective(survey.deadline, now) == survey.status {
        return Ok(survey);
    }
    survey::ActiveModel {
        id: Set(survey.id),
        status: Set(SurveyStatus::Closed),
        updated_at: Set(now),
        ..Default::default()
    }
    .update_without_returning(tx)
    .await?;
    survey.status = SurveyStatus::Closed;
    survey.updated_at = now;
    Ok(survey)
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
