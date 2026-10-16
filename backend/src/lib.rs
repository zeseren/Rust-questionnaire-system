//! 问卷调查系统后端：HTTP 边界 → 业务用例 → 数据访问，领域校验不依赖数据库。

mod admin;
mod auth;
mod config;
pub use questionnaire_logic::domain;
mod entities;
mod error;
mod http;
mod migration;
mod reports;
mod repository;
mod security;
mod submissions;
mod surveys;

use crate::{
    config::Config,
    error::{AppError, AppResult},
    security::Limits,
};
use anyhow::{Context, bail};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    middleware,
    routing::get,
};
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;
use std::{sync::Arc, time::Duration};
use utoipa::openapi::{
    Components,
    security::{ApiKey, ApiKeyValue, SecurityScheme},
};
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(Clone)]
pub(crate) struct AppState {
    pub db: DatabaseConnection,
    pub config: Arc<Config>,
    pub limits: Arc<Limits>,
}

/// 从环境变量启动服务，或执行 migrate / create-admin 子命令。
///
/// # Errors
/// 配置、数据库连接、迁移、管理员初始化或监听失败时返回错误；不自动切库或重试。
pub async fn run() -> anyhow::Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(error) if error.not_found() => {}
        Err(error) => return Err(error).context("无法读取 .env"),
    }
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("questionnaire_api=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
    let config = Config::from_env()?;
    let mut options = ConnectOptions::new(config.database_url.clone());
    options
        .max_connections(10)
        .min_connections(1)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5))
        .sqlx_logging(false);
    let db = Database::connect(options)
        .await
        .context("无法连接 PostgreSQL")?;
    migration::Migrator::up(&db, None)
        .await
        .context("数据库迁移失败")?;

    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "serve".to_owned());
    match (command.as_str(), args.next()) {
        ("migrate", None) => {
            tracing::info!("database migrations completed");
            db.close().await?;
            return Ok(());
        }
        ("create-admin", None) => {
            let username = std::env::var("ADMIN_USERNAME").context("请设置 ADMIN_USERNAME")?;
            let password = std::env::var("ADMIN_PASSWORD").context("请设置 ADMIN_PASSWORD")?;
            auth::create_admin(&db, username, password).await?;
            tracing::info!("administrator created");
            db.close().await?;
            return Ok(());
        }
        ("serve", None) => {}
        _ => bail!("用法: questionnaire-api [serve|migrate|create-admin]"),
    }
    let listen = config.listen;
    let state = AppState {
        db,
        config: Arc::new(config),
        limits: Arc::new(Limits::new()),
    };
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .context("无法监听 HTTP 地址")?;
    tracing::info!(address = %listen, "server ready");
    axum::serve(listener, router(state.clone()))
        .with_graceful_shutdown(shutdown())
        .await?;
    state.db.close().await?;
    Ok(())
}

fn router(state: AppState) -> Router {
    let (router, mut doc) = OpenApiRouter::new()
        .merge(auth::router())
        .merge(surveys::router())
        .merge(submissions::router())
        .merge(reports::router())
        .merge(admin::router())
        .routes(routes!(health))
        .split_for_parts();
    doc.info.title = "问卷调查系统 API".to_owned();
    doc.info.description = Some("所有写请求必须携带 Origin（等于 PUBLIC_ORIGIN）和 X-CSRF-Protection: 1。认证使用 HttpOnly session Cookie。JSON 时间为 RFC3339，时间范围左闭右开；文本为普通文本，前端必须转义渲染。".to_owned());
    doc.components
        .get_or_insert_with(Components::new)
        .add_security_scheme(
            "cookieAuth",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("session"))),
        );
    // 路由与文档共用 handler 注解，文档仅在组装时生成。
    router
        .route(
            "/api/openapi.json",
            get(move || std::future::ready(Json(doc.clone()))),
        )
        .fallback(|| async { AppError::not_found() })
        .method_not_allowed_fallback(|| async {
            AppError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "不支持此请求方法",
            )
        })
        .layer(DefaultBodyLimit::max(crate::domain::MAX_SUBMISSION_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::guard,
        ))
        .with_state(state)
}

#[utoipa::path(get, path = "/health", responses((status = 200, description = "数据库就绪"), (status = 503, body = error::ErrorResponse)))]
async fn health(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    state.db.ping().await.map_err(|_| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
            "数据库尚未就绪",
        )
    })?;
    Ok(Json(serde_json::json!({"status":"ready"})))
}

async fn shutdown() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "shutdown signal failed");
    }
}
