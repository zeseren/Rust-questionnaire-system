//! 共享的归属查询和行锁。所有回收写操作先锁用户，再锁问卷，形成一致的并发顺序。
//!
//! 只读路径共用 `read_snapshot`，列表查询共用 `title_column` 与 `owned_survey_meta`，
//! 避免各模块重复配置隔离级别或把整份 JsonBinary 草稿搬进内存。
//! 已发布版本不可变，`VersionCache` 让提交、报表与公开题面共享同一份编译结果。

use std::{
    collections::HashMap,
    sync::{Arc, PoisonError, RwLock},
};

use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use sea_orm::{
    AccessMode, ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection,
    DatabaseTransaction, EntityTrait, FromQueryResult, IsolationLevel, QueryFilter, QuerySelect,
    Set, TransactionTrait,
    sea_query::{Expr, LockType, SimpleExpr, extension::postgres::PgExpr},
};
use uuid::Uuid;

use crate::{
    auth,
    domain::PreparedDocument,
    entities::{FillingMode, SurveyStatus, survey, user, version},
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

/// 已发布版本的编译结果；版本不可变，克隆只复制共享指针。
#[derive(Clone)]
pub(crate) struct CachedVersion {
    pub id: Uuid,
    pub survey_id: Uuid,
    pub number: i32,
    pub published_at: DateTime<Utc>,
    pub prepared: Arc<PreparedDocument>,
}

/// 只投影缓存所需列的版本行；解码与编译时机由调用方决定。
#[derive(FromQueryResult)]
pub(crate) struct VersionRow {
    id: Uuid,
    number: i32,
    published_at: DateTime<Utc>,
    document: serde_json::Value,
}

impl VersionRow {
    /// 解析并编译发布版本；调用方已通过数据库归属过滤确认版本属于该问卷。
    fn prepare(self, survey_id: Uuid) -> AppResult<CachedVersion> {
        Ok(CachedVersion {
            id: self.id,
            survey_id,
            number: self.number,
            published_at: self.published_at,
            prepared: Arc::new(PreparedDocument::new(serde_json::from_value(
                self.document,
            )?)?),
        })
    }
}

/// 已发布版本的有界只读缓存；锁只在同步方法内持有，绝不跨 `.await`。
pub(crate) struct VersionCache {
    entries: RwLock<HashMap<Uuid, CachedVersion>>,
}

const VERSION_CACHE_CAPACITY: usize = 256;

impl VersionCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// 命中且问卷归属一致时返回共享句柄，避免把其他问卷的版本交给调用方。
    pub(crate) fn get(&self, survey_id: Uuid, version_id: Uuid) -> Option<CachedVersion> {
        let entries = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        entries
            .get(&version_id)
            .filter(|version| version.survey_id == survey_id)
            .cloned()
    }

    /// 写入版本；满容量时淘汰发布时间最早的条目（版本不可变，淘汰任意项都安全）。
    pub(crate) fn insert(&self, version: CachedVersion) {
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.len() >= VERSION_CACHE_CAPACITY
            && !entries.contains_key(&version.id)
            && let Some(oldest) = entries
                .values()
                .min_by_key(|cached| cached.published_at)
                .map(|cached| cached.id)
        {
            entries.remove(&oldest);
        }
        entries.insert(version.id, version);
    }

    /// 解析、编译并写入一条版本行，返回可共享的句柄。
    pub(crate) fn insert_row(&self, row: VersionRow, survey_id: Uuid) -> AppResult<CachedVersion> {
        let version = row.prepare(survey_id)?;
        self.insert(version.clone());
        Ok(version)
    }
}

/// 按问卷归属投影缓存所需的版本列；事务提交与 JSONB 解码由调用方决定。
pub(crate) async fn load_version_row(
    db: &impl ConnectionTrait,
    survey_id: Uuid,
    version_id: Uuid,
) -> AppResult<Option<VersionRow>> {
    Ok(version::Entity::find_by_id(version_id)
        .filter(version::Column::SurveyId.eq(survey_id))
        .select_only()
        .columns([
            version::Column::Id,
            version::Column::Number,
            version::Column::PublishedAt,
            version::Column::Document,
        ])
        .into_model::<VersionRow>()
        .one(db)
        .await?)
}

/// 版本查询结果：缓存命中直接复用编译产物，未命中保留待提交解码的版本行。
pub(crate) enum VersionLookup {
    Hit(CachedVersion),
    Miss(VersionRow),
}

impl VersionLookup {
    /// 统一解析入口：未命中时编译并写入缓存；调用方决定是否已提交只读事务。
    pub(crate) fn resolve(
        self,
        versions: &VersionCache,
        survey_id: Uuid,
    ) -> AppResult<CachedVersion> {
        match self {
            Self::Hit(version) => Ok(version),
            Self::Miss(row) => versions.insert_row(row, survey_id),
        }
    }
}

/// 缓存优先的版本查询：命中零查询；未命中只返回缓存所需列，解码时机交给调用方。
pub(crate) async fn lookup_version(
    db: &impl ConnectionTrait,
    versions: &VersionCache,
    survey_id: Uuid,
    version_id: Uuid,
) -> AppResult<Option<VersionLookup>> {
    if let Some(version) = versions.get(survey_id, version_id) {
        return Ok(Some(VersionLookup::Hit(version)));
    }
    Ok(load_version_row(db, survey_id, version_id)
        .await?
        .map(VersionLookup::Miss))
}

/// 统一加载入口：缓存命中零查询；未命中时投影版本行、编译并写入缓存。
pub(crate) async fn load_cached_version(
    db: &impl ConnectionTrait,
    versions: &VersionCache,
    survey_id: Uuid,
    version_id: Uuid,
) -> AppResult<Option<CachedVersion>> {
    lookup_version(db, versions, survey_id, version_id)
        .await?
        .map(|lookup| lookup.resolve(versions, survey_id))
        .transpose()
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

/// 所有者写入的元数据入口：先锁并重验当前账号，再锁元数据并核对归属。
pub(crate) async fn lock_owned_survey_meta(
    tx: &DatabaseTransaction,
    headers: &HeaderMap,
    owner_id: Uuid,
    id: Uuid,
) -> AppResult<SurveyMetaRow> {
    lock_session_user(tx, headers, owner_id).await?;
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
