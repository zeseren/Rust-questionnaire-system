//! 测试共用夹具：断开连接的 AppState、实体模型、JSON 断言与原始 HTTP 往返。
//!
//! 全部夹具只在内存中运行：`DatabaseConnection::default()` 是显式的 Disconnected 连接，
//! 所以任何真的走到数据库的路径都会立刻失败，而不是悄悄连上某个本地实例。

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use sea_orm::DatabaseConnection;
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use uuid::Uuid;

use crate::{
    AppState,
    config::Config,
    domain::Document,
    entities::{SurveyStatus, UserRole, survey, user},
    error::AppError,
    repository::VersionCache,
    security::Limits,
};

/// 测试配置：限流上限取很小且彼此不同的值，便于用最少请求验证桶的边界。
pub(crate) fn test_config() -> Config {
    Config {
        database_url: "postgres://questionnaire:questionnaire@127.0.0.1:5432/questionnaire"
            .to_owned(),
        listen: "127.0.0.1:3000".parse().expect("测试监听地址合法"),
        public_origin: "http://localhost:3000".to_owned(),
        cookie_secure: false,
        session_ttl_seconds: 3_600,
        auth_requests_per_minute: 2,
        submissions_per_minute: 3,
    }
}

/// 默认测试状态：配置见 [`test_config`]，数据库为显式断开连接的占位对象。
pub(crate) fn test_state() -> AppState {
    AppState {
        db: DatabaseConnection::default(),
        config: Arc::new(test_config()),
        limits: Arc::new(Limits::new()),
        versions: Arc::new(VersionCache::new()),
    }
}

/// 最小合法题面；管理 DTO 只反序列化草稿，不重复发布校验。
pub(crate) fn survey_document() -> Document {
    Document {
        logic: None,
        title: "测试问卷".to_owned(),
        description: String::new(),
        purpose: "测试用途".to_owned(),
        thank_you: "谢谢".to_owned(),
        questions: Vec::new(),
    }
}

/// 未发布的问卷实体：revision 从 1 开始，没有当前版本。
pub(crate) fn survey_model() -> survey::Model {
    let now = Utc::now();
    survey::Model {
        id: Uuid::new_v4(),
        owner_id: Uuid::new_v4(),
        status: SurveyStatus::Unpublished,
        mode: None,
        deadline: None,
        taken_down: false,
        draft: serde_json::to_value(survey_document()).expect("草稿必须可序列化"),
        revision: 1,
        current_version_id: None,
        created_at: now,
        updated_at: now,
    }
}

/// 普通用户实体；密码哈希只用于确认对外 DTO 不会带出它。
pub(crate) fn user_model() -> user::Model {
    user::Model {
        id: Uuid::new_v4(),
        username: "alice".to_owned(),
        password_hash: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$hash".to_owned(),
        role: UserRole::User,
        disabled: false,
        created_at: Utc::now(),
    }
}

/// 构造一个只带方法与头部、空请求体的提取器输入。
pub(crate) fn raw_request(method: &str, uri: &str, headers: &[(&str, &str)]) -> Request {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder.body(Body::empty()).expect("测试请求必须可构造")
}

/// 把响应拆成状态码、响应头与 JSON 体；非 JSON 或空体分别视为解析失败与 null。
pub(crate) async fn response_json(response: Response) -> (StatusCode, HeaderMap, Value) {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("测试响应体必须可读取");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("测试响应体必须是 JSON")
    };
    (status, headers, body)
}

/// 错误响应的统一出口：测试只通过 `IntoResponse` 观察状态码与错误契约。
pub(crate) async fn error_json(error: AppError) -> (StatusCode, HeaderMap, Value) {
    response_json(error.into_response()).await
}

/// 建立真实 TCP 连接，发送原始 HTTP 报文并解析响应。
///
/// 端到端协议行为（方法匹配、fallback、中间件统一响应头）无法用提取器单测覆盖，
/// 因此这里用一次性监听端口与 `axum::serve` 走完整链路；每个用例独占一个端口。
pub(crate) async fn send_raw_http(app: Router, request: &str) -> (StatusCode, HeaderMap, Value) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("绑定测试端口");
    let address = listener.local_addr().expect("取得测试端口");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let response = read_response(address, request).await;
    server.abort();
    response
}

async fn read_response(address: SocketAddr, request: &str) -> (StatusCode, HeaderMap, Value) {
    let mut stream = TcpStream::connect(address).await.expect("连接测试服务");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("发送原始请求");
    stream.flush().await.expect("刷新原始请求");
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await.expect("读取原始响应");
        if read == 0 {
            break;
        }
        received.extend_from_slice(&chunk[..read]);
        if let Some(response) = parse_response(&received) {
            return response;
        }
    }
    panic!("连接关闭前没有收到完整响应：{} 字节", received.len());
}

/// 按 `Content-Length` 判断报文是否完整；只接受 JSON 响应体。
fn parse_response(raw: &[u8]) -> Option<(StatusCode, HeaderMap, Value)> {
    let split = raw.windows(4).position(|window| window == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).expect("响应头必须是 UTF-8");
    let body_start = split + 4;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse::<u16>()
        .ok()
        .and_then(|code| StatusCode::from_u16(code).ok())?;
    let mut headers = HeaderMap::new();
    for line in lines {
        let (name, value) = line.split_once(':')?;
        headers.append(
            HeaderName::from_bytes(name.trim().as_bytes()).ok()?,
            HeaderValue::from_str(value.trim()).ok()?,
        );
    }
    let length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if raw.len() < body_start + length {
        return None;
    }
    let body = &raw[body_start..body_start + length];
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(body).expect("测试只请求 JSON 响应")
    };
    Some((status, headers, value))
}
