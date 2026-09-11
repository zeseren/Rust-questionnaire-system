//! 同源写请求校验及单实例固定窗口限流，不收集 IP、设备指纹或答案内容。

use crate::{
    AppState,
    error::{AppError, AppResult},
};
use axum::{
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::Response,
};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

pub(crate) struct Limits {
    auth: Mutex<Window>,
    submissions: Mutex<Window>,
}

struct Window {
    started: Instant,
    count: u32,
}

impl Limits {
    pub fn new() -> Self {
        Self {
            auth: Mutex::new(Window {
                started: Instant::now(),
                count: 0,
            }),
            submissions: Mutex::new(Window {
                started: Instant::now(),
                count: 0,
            }),
        }
    }
}

fn admit(window: &Mutex<Window>, maximum: u32) -> AppResult<()> {
    let mut window = window.lock().map_err(AppError::internal)?;
    if window.started.elapsed() >= Duration::from_secs(60) {
        window.started = Instant::now();
        window.count = 0;
    }
    if window.count >= maximum {
        return Err(AppError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "请求过于频繁，请稍后重试",
        ));
    }
    window.count += 1;
    Ok(())
}

pub(crate) async fn guard(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> AppResult<Response> {
    let method = request.method();
    if !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        let origin = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok());
        let marker = request
            .headers()
            .get("x-csrf-protection")
            .and_then(|value| value.to_str().ok());
        // 精确 Origin + 非简单请求头 + 不开放 CORS，阻止跨站表单和跨站脚本携带 Cookie 写入。
        if origin != Some(state.config.public_origin.as_str()) || marker != Some("1") {
            return Err(AppError::new(
                StatusCode::FORBIDDEN,
                "csrf_rejected",
                "写请求必须来自配置的同源站点并携带 X-CSRF-Protection: 1",
            ));
        }
        let path = request.uri().path();
        if path.starts_with("/api/auth/") {
            admit(&state.limits.auth, state.config.auth_requests_per_minute)?;
        } else if *method == Method::POST
            && path.starts_with("/api/public/surveys/")
            && path.ends_with("/responses")
        {
            admit(
                &state.limits.submissions,
                state.config.submissions_per_minute,
            )?;
        }
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}
