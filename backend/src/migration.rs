//! 唯一的数据库结构入口。sea-orm-migration 记录版本，原生 SQL 表达 PostgreSQL 约束。

use sea_orm_migration::prelude::*;

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
        manager
            .get_connection()
            .execute_unprepared(include_str!("schema.sql"))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "DROP TABLE audits, responses; ALTER TABLE surveys DROP CONSTRAINT survey_current_version; DROP TABLE versions, surveys, sessions, users;"
        ).await?;
        Ok(())
    }
}
