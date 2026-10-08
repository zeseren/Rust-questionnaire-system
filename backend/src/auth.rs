//! 账号与 Session：浏览器保存随机秘密，数据库只保存秘密的 SHA-256 摘要。
//! 密码运算放到阻塞线程池；账号使用 FOR NO KEY UPDATE，既协调资格变更，也兼容外键检查。

use argon2::{
    Argon2,
    password_hash::{Error as PasswordError, PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
};
use chrono::{Duration, Utc};
use cookie::{Cookie, SameSite};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    FromQueryResult, IntoActiveModel, QueryFilter, QuerySelect, QueryTrait, Set, SqlErr,
    TransactionTrait, sea_query::LockType,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    AppState,
    entities::{session, user},
    error::{AppError, AppResult, ErrorResponse},
    http::ApiJson,
};

/// 认证提取器只携带后续授权所需的信息，不携带密码哈希或会话秘密。
#[derive(Clone, Debug, FromQueryResult)]
pub(crate) struct AuthUser {
    pub id: Uuid,
    pub username: String,
    pub role: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> AppResult<Self> {
        authenticate(&parts.headers, &state.db).await
    }
}

/// 注册只接收用户名和密码；传入 role 等额外字段会被明确拒绝。
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegisterRequest {
    /// 3—32 个 ASCII 字母、数字或下划线，保存时统一为小写。
    #[schema(min_length = 3, max_length = 32, pattern = "^[A-Za-z0-9_]{3,32}$")]
    pub username: String,
    /// 8—128 个 Unicode 标量值，不截断、不去除空白。
    #[schema(min_length = 8, max_length = 128, format = Password, write_only = true)]
    pub password: String,
}

/// 登录失败不区分用户名不存在、密码错误或账号已禁用。
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginRequest {
    #[schema(min_length = 3, max_length = 32, pattern = "^[A-Za-z0-9_]{3,32}$")]
    pub username: String,
    #[schema(min_length = 8, max_length = 128, format = Password, write_only = true)]
    pub password: String,
}

/// 修改成功后所有设备都需要重新登录，包括当前请求的 Session。
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangePasswordRequest {
    #[schema(min_length = 8, max_length = 128, format = Password, write_only = true)]
    pub old_password: String,
    #[schema(min_length = 8, max_length = 128, format = Password, write_only = true)]
    pub new_password: String,
}

/// 账号对外基本资料；不直接序列化 ORM 用户实体。
#[derive(Serialize, ToSchema)]
pub(crate) struct UserProfile {
    pub id: Uuid,
    pub username: String,
    /// user 为普通用户，admin 为受控创建的管理员。
    pub role: String,
}

impl From<user::Model> for UserProfile {
    fn from(user: user::Model) -> Self {
        Self {
            id: user.id,
            username: user.username,
            role: user.role,
        }
    }
}

impl From<AuthUser> for UserProfile {
    fn from(user: AuthUser) -> Self {
        Self {
            id: user.id,
            username: user.username,
            role: user.role,
        }
    }
}

pub(crate) fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(register))
        .routes(routes!(login))
        .routes(routes!(logout))
        .routes(routes!(me))
        .routes(routes!(change_password))
}

/// 注册普通账号。注册本身不建立登录会话。
#[utoipa::path(
    post, path = "/api/auth/register", tag = "账号",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "普通账号创建成功，请继续登录", body = UserProfile),
        (status = 409, description = "用户名已被使用", body = ErrorResponse),
        (status = 422, description = "请求字段或格式不合法", body = ErrorResponse),
        (status = 403, description = "请求来源不符合 CSRF 策略", body = ErrorResponse),
        (status = 429, description = "请求过于频繁", body = ErrorResponse),
        (status = 500, description = "服务内部错误", body = ErrorResponse)
    )
)]
async fn register(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<RegisterRequest>,
) -> AppResult<(StatusCode, Json<UserProfile>)> {
    let user = create_user(&state.db, request.username, request.password, "user").await?;
    Ok((StatusCode::CREATED, Json(user.into())))
}

/// 登录成功后只通过 HttpOnly Cookie 发送新的会话秘密。
#[utoipa::path(
    post, path = "/api/auth/login", tag = "账号",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "登录成功，旧 Cookie 对应会话已撤销", body = UserProfile,
            headers(("Set-Cookie" = String, description = "session；HttpOnly、SameSite=Lax、Path=/；HTTPS 使用 Secure"))),
        (status = 401, description = "用户名或密码错误", body = ErrorResponse),
        (status = 422, description = "请求结构不合法", body = ErrorResponse),
        (status = 403, description = "请求来源不符合 CSRF 策略", body = ErrorResponse),
        (status = 429, description = "请求过于频繁", body = ErrorResponse),
        (status = 500, description = "服务内部错误", body = ErrorResponse)
    )
)]
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<LoginRequest>,
) -> AppResult<([(header::HeaderName, HeaderValue); 1], Json<UserProfile>)> {
    let username = normalize_username(request.username).map_err(|_| invalid_credentials())?;
    validate_password(&request.password).map_err(|_| invalid_credentials())?;
    let candidate = user::Entity::find()
        .filter(user::Column::Username.eq(username))
        .one(&state.db)
        .await?
        .ok_or_else(invalid_credentials)?;
    if !verify_password(request.password, candidate.password_hash.clone()).await? {
        return Err(invalid_credentials());
    }
    if candidate.disabled {
        return Err(invalid_credentials());
    }

    let token = new_session_token()?;
    let cookie = session_cookie(
        &token,
        state.config.session_ttl_seconds,
        state.config.cookie_secure,
    )?;

    let tx = state.db.begin().await?;
    let current = user::Entity::find_by_id(candidate.id)
        .lock(LockType::NoKeyUpdate)
        .one(&tx)
        .await?
        .ok_or_else(invalid_credentials)?;
    if current.disabled || current.password_hash != candidate.password_hash {
        return Err(invalid_credentials());
    }
    let lifetime = Duration::try_seconds(state.config.session_ttl_seconds)
        .filter(|duration| *duration > Duration::zero())
        .ok_or_else(|| AppError::internal("invalid session lifetime"))?;
    let expires_at = Utc::now()
        .checked_add_signed(lifetime)
        .ok_or_else(|| AppError::internal("session expiry overflow"))?;
    // 旧 Cookie 本身就是撤销该会话的凭证；无需同时锁定另一个账号。
    if let Some(old_token) = session_token(&headers) {
        session::Entity::delete_by_id(hash_secret(old_token))
            .exec(&tx)
            .await?;
    }
    session::ActiveModel {
        id_hash: Set(hash_secret(&token)),
        user_id: Set(current.id),
        expires_at: Set(expires_at),
    }
    .insert(&tx)
    .await?;
    tx.commit().await?;
    Ok(([(header::SET_COOKIE, cookie)], Json(current.into())))
}

/// 退出可重复调用；即使会话已经过期，也删除旧记录并清除浏览器 Cookie。
#[utoipa::path(
    post, path = "/api/auth/logout", tag = "账号",
    responses(
        (status = 204, description = "会话已撤销并清除 Cookie",
            headers(("Set-Cookie" = String, description = "清除 session Cookie，Max-Age=0"))),
        (status = 403, description = "请求来源不符合 CSRF 策略", body = ErrorResponse),
        (status = 500, description = "服务内部错误", body = ErrorResponse)
    )
)]
async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<(StatusCode, [(header::HeaderName, HeaderValue); 1])> {
    let cookie = session_cookie("", 0, state.config.cookie_secure)?;
    revoke_session(&state.db, session_token(&headers)).await?;
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]))
}

/// 获取当前有效账号的基本资料。
#[utoipa::path(
    get, path = "/api/auth/me", tag = "账号", security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "当前账号", body = UserProfile),
        (status = 401, description = "会话失效或账号被禁用", body = ErrorResponse),
        (status = 500, description = "服务内部错误", body = ErrorResponse)
    )
)]
async fn me(user: AuthUser) -> Json<UserProfile> {
    Json(user.into())
}

/// 验证旧密码后更新密码，并在同一事务中撤销全部登录会话。
#[utoipa::path(
    put, path = "/api/auth/password", tag = "账号", security(("cookieAuth" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (status = 204, description = "密码已修改，所有会话失效，需要重新登录",
            headers(("Set-Cookie" = String, description = "清除当前 session Cookie，Max-Age=0"))),
        (status = 401, description = "会话失效、账号禁用或旧密码错误", body = ErrorResponse),
        (status = 422, description = "密码长度或请求字段不合法", body = ErrorResponse),
        (status = 403, description = "请求来源不符合 CSRF 策略", body = ErrorResponse),
        (status = 500, description = "服务内部错误", body = ErrorResponse)
    )
)]
async fn change_password(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ChangePasswordRequest>,
) -> AppResult<(StatusCode, [(header::HeaderName, HeaderValue); 1])> {
    validate_password(&request.old_password)?;
    validate_password(&request.new_password)?;
    let candidate = user::Entity::find_by_id(user.id)
        .one(&state.db)
        .await?
        .ok_or_else(AppError::unauthorized)?;
    if !verify_password(request.old_password, candidate.password_hash.clone()).await? {
        return Err(invalid_credentials());
    }
    let new_hash = hash_password(request.new_password).await?;
    let cookie = session_cookie("", 0, state.config.cookie_secure)?;

    let tx = state.db.begin().await?;
    let current = user::Entity::find_by_id(user.id)
        .lock(LockType::NoKeyUpdate)
        .one(&tx)
        .await?
        .ok_or_else(AppError::unauthorized)?;
    let authenticated = authenticate(&headers, &tx).await?;
    // 锁前算出的验证结果，只能用于同一个仍有效的会话和没有变化的密码版本。
    if authenticated.id != user.id
        || current.disabled
        || current.password_hash != candidate.password_hash
    {
        return Err(AppError::unauthorized());
    }
    let mut updated = current.into_active_model();
    updated.password_hash = Set(new_hash);
    updated.update(&tx).await?;
    session::Entity::delete_many()
        .filter(session::Column::UserId.eq(user.id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]))
}

/// 用同一条查询检查会话有效期与账号状态，既可传连接，也可传当前业务事务。
/// 涉及资格变更的业务应先持有账号行锁，再调用本函数重验 Session。
pub(crate) async fn authenticate(
    headers: &HeaderMap,
    db: &impl ConnectionTrait,
) -> AppResult<AuthUser> {
    let token = session_token(headers).ok_or_else(AppError::unauthorized)?;
    let active_session = session::Entity::find()
        .select_only()
        .column(session::Column::UserId)
        .filter(session::Column::IdHash.eq(hash_secret(token)))
        .filter(session::Column::ExpiresAt.gt(Utc::now()))
        .into_query();
    user::Entity::find()
        .select_only()
        .columns([user::Column::Id, user::Column::Username, user::Column::Role])
        .filter(user::Column::Id.in_subquery(active_session))
        .filter(user::Column::Disabled.eq(false))
        .into_model::<AuthUser>()
        .one(db)
        .await?
        .ok_or_else(AppError::unauthorized)
}

/// 仅供受控 CLI 初始化：只插入新的管理员，不覆盖或升级任何已有账号。
pub(crate) async fn create_admin(
    db: &DatabaseConnection,
    username: String,
    password: String,
) -> AppResult<()> {
    create_user(db, username, password, "admin").await?;
    Ok(())
}

async fn create_user(
    db: &DatabaseConnection,
    username: String,
    password: String,
    role: &'static str,
) -> AppResult<user::Model> {
    let username = normalize_username(username)?;
    validate_password(&password)?;
    let password_hash = hash_password(password).await?;
    user::ActiveModel {
        id: Set(Uuid::new_v4()),
        username: Set(username),
        password_hash: Set(password_hash),
        role: Set(role.to_owned()),
        disabled: Set(false),
        created_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .map_err(|error| match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => {
            AppError::conflict("username_taken", "用户名已被使用")
        }
        _ => AppError::from(error),
    })
}

fn normalize_username(mut username: String) -> AppResult<String> {
    if !(3..=32).contains(&username.len())
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(AppError::validation(
            "用户名须为 3—32 个 ASCII 字母、数字或下划线",
        ));
    }
    username.make_ascii_lowercase();
    Ok(username)
}

fn validate_password(password: &str) -> AppResult<()> {
    if !(8..=128).contains(&password.chars().take(129).count()) {
        return Err(AppError::validation("密码长度须为 8—128 个字符"));
    }
    Ok(())
}

fn invalid_credentials() -> AppError {
    AppError::new(
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
        "用户名或密码错误",
    )
}

async fn hash_password(password: String) -> AppResult<String> {
    tokio::task::spawn_blocking(move || {
        // Argon2id v19 自动生成随机盐；随机源失败仍通过统一错误出口返回。
        let hash: PasswordHash = Argon2::default()
            .hash_password(password.as_bytes())
            .map_err(|error| AppError::internal(error))?;
        Ok(hash.to_string())
    })
    .await
    .map_err(|error| AppError::internal(error))?
}

async fn verify_password(password: String, encoded_hash: String) -> AppResult<bool> {
    tokio::task::spawn_blocking(move || {
        let hash = PasswordHash::new(&encoded_hash).map_err(|error| AppError::internal(error))?;
        match Argon2::default().verify_password(password.as_bytes(), &hash) {
            Ok(()) => Ok(true),
            Err(PasswordError::PasswordInvalid) => Ok(false),
            Err(error) => Err(AppError::internal(error)),
        }
    })
    .await
    .map_err(|error| AppError::internal(error))?
}

fn new_session_token() -> AppResult<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| AppError::internal(error))?;
    Ok(hex::encode(bytes))
}

/// 高熵秘密可以使用快速摘要；用户密码必须使用上面的 Argon2id，不能用此函数。
pub(crate) fn hash_secret(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for cookie in Cookie::split_parse(header.to_str().ok()?) {
            let Ok(cookie) = cookie else {
                continue;
            };
            if cookie.name() != "session" {
                continue;
            }
            let value = cookie.value_raw()?;
            // 重复同名 Cookie 的含义不明确，不以其中任意一个作为登录身份。
            if found.is_some()
                || value.len() != 64
                || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return None;
            }
            found = Some(value);
        }
    }
    found
}

fn session_cookie(token: &str, max_age: i64, secure: bool) -> AppResult<HeaderValue> {
    let cookie = Cookie::build(("session", token))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(cookie::time::Duration::seconds(max_age))
        .secure(secure)
        .build();
    HeaderValue::from_str(&cookie.to_string()).map_err(|error| AppError::internal(error))
}

async fn revoke_session(db: &DatabaseConnection, token: Option<&str>) -> AppResult<()> {
    let Some(token) = token else { return Ok(()) };
    let id_hash = hash_secret(token);
    let tx = db.begin().await?;
    if let Some(existing) = session::Entity::find_by_id(id_hash.clone())
        .one(&tx)
        .await?
    {
        // NO KEY UPDATE 与提交方的 FOR SHARE 互斥，但不会阻塞审计外键的 KEY SHARE。
        user::Entity::find_by_id(existing.user_id)
            .lock(LockType::NoKeyUpdate)
            .one(&tx)
            .await?;
        session::Entity::delete_by_id(id_hash).exec(&tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
