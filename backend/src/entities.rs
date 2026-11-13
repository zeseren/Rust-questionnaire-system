//! `SeaORM` 实体只映射数据库。HTTP 响应用各业务模块的 DTO，避免泄露密码和提交秘密。
//!
//! 状态、模式、角色与审计动作直接以强类型枚举映射文本列：字符串编解码只由
//! `DeriveActiveEnum` 负责，业务代码不再手写 `match` 转换。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 问卷业务状态；截止时间到期在读取时物化为已关闭，不依赖后台任务。
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, EnumIter, DeriveActiveEnum,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
pub(crate) enum SurveyStatus {
    Unpublished,
    Collecting,
    Closed,
    Archived,
}

impl SurveyStatus {
    /// 回收中且已到截止时间的问卷在读取时等价于已关闭；其余状态保持原值。
    pub(crate) fn effective(self, deadline: Option<DateTimeUtc>, now: DateTimeUtc) -> Self {
        if self == Self::Collecting && deadline.is_some_and(|limit| limit <= now) {
            Self::Closed
        } else {
            self
        }
    }
}

/// `anonymous` 不关联账号；`login_once` 对同一问卷按账号跨版本限交一次。
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, EnumIter, DeriveActiveEnum,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
pub(crate) enum FillingMode {
    Anonymous,
    LoginOnce,
}

/// 账号角色；注册只能创建 user，admin 仅由受控 CLI 创建。
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, EnumIter, DeriveActiveEnum,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
pub(crate) enum UserRole {
    User,
    Admin,
}

/// 管理员治理动作；`take_down` 沿用既有数据库 CHECK 与前端展示契约。
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, EnumIter, DeriveActiveEnum,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
pub(crate) enum AuditAction {
    DisableUser,
    EnableUser,
    #[serde(rename = "take_down")]
    #[sea_orm(string_value = "take_down")]
    TakedownSurvey,
    RestoreSurvey,
}

/// 一次正式提交的流程终点；筛选结束同样落库，但不计入有效完成样本。
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, EnumIter, DeriveActiveEnum,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
pub(crate) enum Outcome {
    Complete,
    ScreenOut,
}

impl From<crate::domain::FlowOutcome> for Outcome {
    fn from(outcome: crate::domain::FlowOutcome) -> Self {
        match outcome {
            crate::domain::FlowOutcome::Complete => Self::Complete,
            crate::domain::FlowOutcome::ScreenOut => Self::ScreenOut,
        }
    }
}

pub(crate) mod user {
    use super::UserRole;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub username: String,
        pub password_hash: String,
        pub role: UserRole,
        pub disabled: bool,
        pub created_at: DateTimeUtc,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod session {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "sessions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id_hash: String,
        pub user_id: Uuid,
        pub expires_at: DateTimeUtc,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod survey {
    use super::{FillingMode, SurveyStatus};
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "surveys")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub owner_id: Uuid,
        pub status: SurveyStatus,
        pub mode: Option<FillingMode>,
        pub deadline: Option<DateTimeUtc>,
        pub taken_down: bool,
        #[sea_orm(column_type = "JsonBinary")]
        pub draft: Json,
        pub revision: i64,
        pub current_version_id: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod version {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "versions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub survey_id: Uuid,
        pub number: i32,
        #[sea_orm(column_type = "JsonBinary")]
        pub document: Json,
        pub published_at: DateTimeUtc,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod response {
    use super::Outcome;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "responses")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub survey_id: Uuid,
        pub version_id: Uuid,
        pub respondent_id: Option<Uuid>,
        pub scope: String,
        pub key_hash: String,
        pub payload_hash: String,
        #[sea_orm(column_type = "JsonBinary")]
        pub answers: Json,
        pub outcome: Outcome,
        #[sea_orm(column_type = "JsonBinary")]
        pub question_states: Json,
        pub submitted_at: DateTimeUtc,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod audit {
    use super::AuditAction;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "audits")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub actor_id: Uuid,
        pub action: AuditAction,
        pub target_id: Uuid,
        pub reason: String,
        pub created_at: DateTimeUtc,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
