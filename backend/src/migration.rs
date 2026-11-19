//! 唯一的数据库结构入口。sea-orm-migration 记录版本，原生 SQL 表达 PostgreSQL 约束。
//! `schema.sql` 的归一化指纹随库保存，每次启动校验：旧库缺列不会拖到运行期才 500。

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;
use sha2::{Digest, Sha256};

const SCHEMA: &str = include_str!("schema.sql");
/// 结构指纹记账表；它不属于业务模型，因此不写进 `schema.sql`。
const META_DDL: &str = "CREATE TABLE IF NOT EXISTS schema_meta (digest text PRIMARY KEY)";
/// 已确认库与 `schema.sql` 对齐时，用它重新记录基线。
const REBASELINE_ENV: &str = "SCHEMA_REBASELINE";

pub(crate) struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(InitialSchema)]
    }
}

#[derive(DeriveMigrationName)]
struct InitialSchema;

#[async_trait::async_trait]
impl MigrationTrait for InitialSchema {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let connection = manager.get_connection();
        connection.execute_unprepared(SCHEMA).await?;
        record_digest(connection, &expected_digest()).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "DROP TABLE schema_meta; DROP TABLE audits, responses; ALTER TABLE surveys DROP CONSTRAINT survey_current_version; DROP TABLE versions, surveys, sessions, users;"
        ).await?;
        Ok(())
    }
}

/// `schema.sql` 去掉注释与空行后的摘要：只表达结构，不随排版或注释变化。
fn digest_of(sql: &str) -> String {
    let normalized = sql
        .lines()
        .map(|line| line.split("--").next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    hex::encode(Sha256::digest(normalized.as_bytes()))
}

fn expected_digest() -> String {
    digest_of(SCHEMA)
}

/// 校验数据库结构与编译期 `schema.sql` 一致。
///
/// # Errors
/// 指纹缺失（由旧版本创建的库）或与当前 `schema.sql` 不一致时返回错误，
/// 提示重建开发库，或在确认结构已对齐后重新记录基线。
pub(crate) async fn verify_schema<C: ConnectionTrait>(connection: &C) -> Result<(), DbErr> {
    let expected = expected_digest();
    let stored = stored_digest(connection).await?;
    if stored.as_deref() == Some(expected.as_str()) {
        return Ok(());
    }
    if rebaseline() {
        record_digest(connection, &expected).await?;
        tracing::warn!(?stored, current = %expected, "数据库结构指纹已重新记录");
        return Ok(());
    }
    Err(DbErr::Custom(match stored {
        Some(_) => "数据库结构指纹与 schema.sql 不一致：执行 ./start.ps1 -ResetDb 重建开发库；\
             已手动对齐结构时，设置 SCHEMA_REBASELINE=1 后重新启动以记录新基线"
            .to_owned(),
        None => "数据库缺少结构指纹（由旧版本创建）：确认结构与 schema.sql 对齐后，\
             设置 SCHEMA_REBASELINE=1 重新启动，或执行 ./start.ps1 -ResetDb 重建"
            .to_owned(),
    }))
}

async fn stored_digest<C: ConnectionTrait>(connection: &C) -> Result<Option<String>, DbErr> {
    let present = connection
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT to_regclass('public.schema_meta') IS NOT NULL".to_owned(),
        ))
        .await?
        .and_then(|row| row.try_get_by::<bool, _>(0).ok())
        .unwrap_or(false);
    if !present {
        return Ok(None);
    }
    Ok(connection
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT digest FROM schema_meta LIMIT 1".to_owned(),
        ))
        .await?
        .and_then(|row| row.try_get_by::<String, _>(0).ok()))
}

async fn record_digest<C: ConnectionTrait>(connection: &C, digest: &str) -> Result<(), DbErr> {
    connection.execute_unprepared(META_DDL).await?;
    connection
        .execute_unprepared("DELETE FROM schema_meta")
        .await?;
    connection
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO schema_meta (digest) VALUES ($1)",
            [digest.to_owned().into()],
        ))
        .await?;
    Ok(())
}

fn rebaseline() -> bool {
    std::env::var_os(REBASELINE_ENV).is_some_and(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::digest_of;

    #[test]
    fn 结构指纹忽略注释与空行() {
        let base = "CREATE TABLE a (id uuid);\nCREATE INDEX a_id ON a(id);";
        let decorated = "-- 说明\n\n  CREATE TABLE a (id uuid);  \n\n-- 另一条说明\nCREATE INDEX a_id ON a(id);\n";
        assert_eq!(digest_of(base), digest_of(decorated));
    }

    #[test]
    fn 结构指纹随结构变化() {
        let base = "CREATE TABLE a (id uuid);";
        assert_ne!(
            digest_of(base),
            digest_of("CREATE TABLE a (id uuid, extra text);")
        );
        assert_ne!(digest_of(base), digest_of("CREATE TABLE b (id uuid);"));
        assert_eq!(digest_of(base).len(), 64, "SHA-256 十六进制摘要");
    }
}
