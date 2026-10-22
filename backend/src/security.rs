//! 同源写请求校验及单实例固定窗口限流，不收集 IP、设备指纹或答案内容。

use crate::{
    AppState,
    error::{AppError, AppResult},
};
use axum::{
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

pub(crate) struct Limits {
    pub(crate) auth: Mutex<Window>,
    pub(crate) submissions: Mutex<Window>,
}

/// 单桶的固定窗口计数；测试可直接构造并注入合成时刻。
pub(crate) struct Window {
    pub(crate) started: Instant,
    pub(crate) count: u32,
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
    admit_at(window, maximum, Instant::now())
}

/// 在给定的当前时刻执行窗口重置与计数；纯逻辑便于测试 60 秒窗口的精确边界。
pub(crate) fn admit_at(window: &Mutex<Window>, maximum: u32, now: Instant) -> AppResult<()> {
    // 临界区只有计数赋值，中毒时数据仍然完整；恢复使用，避免此后每个请求都返回 500。
    let mut window = window.lock().unwrap_or_else(PoisonError::into_inner);
    if now.saturating_duration_since(window.started) >= Duration::from_secs(60) {
        window.started = now;
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

/// 统一请求闸门。CSRF 与限流拒绝也经过同一出口，所有响应都带 no-store 和 nosniff。
pub(crate) async fn guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let mut response = match check(&state, &request) {
        Ok(()) => next.run(request).await,
        Err(error) => error.into_response(),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

pub(crate) fn check(state: &AppState, request: &Request) -> AppResult<()> {
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
    Ok(())
}
