//! SeaORM 实体只映射数据库。HTTP 响应用各业务模块的 DTO，避免泄露密码和提交秘密。

pub(crate) mod user {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub username: String,
        pub password_hash: String,
        pub role: String,
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
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "surveys")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub owner_id: Uuid,
        pub status: String,
        pub mode: Option<String>,
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
        pub submitted_at: DateTimeUtc,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod audit {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "audits")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub actor_id: Uuid,
        pub action: String,
        pub target_id: Uuid,
        pub reason: String,
        pub created_at: DateTimeUtc,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
