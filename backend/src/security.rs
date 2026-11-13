//! 同源写请求校验及按客户端分桶的固定窗口限流。
//!
//! 内存中保留每日随机盐，桶键只保存 `SHA256(salt || client_ip)`（提交桶再拼问卷 ID），
//! 因此不落库、不出日志，也不会把 IP 明文留在进程状态里；跨日后旧键自然失效并整表清空。

use crate::{
    AppState,
    error::{AppError, AppResult},
};
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::{NaiveDate, Utc};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    hash::Hash,
    net::{IpAddr, SocketAddr},
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};
use uuid::Uuid;

/// 固定窗口长度：一分钟内的同一桶最多 `maximum` 次。
const WINDOW: Duration = Duration::from_secs(60);
/// 单桶映射的条目上限；超过后先淘汰过期窗口，仍超出时整体清空，避免内存无界增长。
const MAX_TRACKED_BUCKETS: usize = 10_000;

pub(crate) struct Limits {
    /// 认证入口按客户端 IP 分桶。
    pub(crate) auth: Mutex<HashMap<[u8; 32], Window>>,
    /// 公开提交按（客户端 IP 摘要，问卷 ID）分桶，避免一个热门问卷挤占其它问卷配额。
    pub(crate) submissions: Mutex<HashMap<([u8; 32], Uuid), Window>>,
    salt: Mutex<Salt>,
}

/// 单桶的固定窗口计数；测试可直接构造并注入合成时刻。
pub(crate) struct Window {
    pub(crate) started: Instant,
    pub(crate) count: u32,
}

struct Salt {
    bytes: [u8; 32],
    day: NaiveDate,
}

impl Limits {
    pub fn new() -> Self {
        Self {
            auth: Mutex::new(HashMap::new()),
            submissions: Mutex::new(HashMap::new()),
            salt: Mutex::new(Salt {
                // 首次使用时才取随机源；`Limits::new` 因此保持不可失败。
                bytes: [0; 32],
                day: NaiveDate::MIN,
            }),
        }
    }

    /// 当日随机盐：跨自然日（UTC）重新生成并清空全部旧桶。
    fn daily_salt(&self) -> AppResult<[u8; 32]> {
        let (bytes, rotated) = {
            let mut salt = self.salt.lock().unwrap_or_else(PoisonError::into_inner);
            let today = Utc::now().date_naive();
            let rotated = salt.day != today;
            if rotated {
                getrandom::fill(&mut salt.bytes).map_err(|error| AppError::internal(error))?;
                salt.day = today;
            }
            (salt.bytes, rotated)
        };
        if rotated {
            self.auth
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            self.submissions
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
        }
        Ok(bytes)
    }
}

/// 桶键只暴露加盐摘要；同一 IP 在当日内的多次请求得到相同键。
fn bucket_key(salt: &[u8; 32], client: Option<IpAddr>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(salt);
    match client.as_ref().map(IpAddr::to_canonical) {
        // IPv4-mapped IPv6 已由 `to_canonical` 归一化为 IPv4，直接哈希原始字节避免堆分配。
        Some(IpAddr::V4(v4)) => {
            hasher.update(b"v4");
            hasher.update(v4.octets());
        }
        Some(IpAddr::V6(v6)) => {
            hasher.update(b"v6");
            hasher.update(v6.octets());
        }
        // 单元测试或未启用 ConnectInfo 的调用方没有对端地址；统一哨兵键同样受限流约束。
        None => hasher.update(b"unknown-connection"),
    }
    hasher.finalize().into()
}

/// 只在本实例确实位于可信反向代理之后时读取转发头，否则客户端可伪造 IP 绕过限流。
fn client_ip(request: &Request, trust_proxy: bool) -> Option<IpAddr> {
    if trust_proxy {
        let forwarded = request
            .headers()
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .find_map(|part| part.parse::<IpAddr>().ok())
            });
        if forwarded.is_some() {
            return forwarded;
        }
        let real = request
            .headers()
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<IpAddr>().ok());
        if real.is_some() {
            return real;
        }
    }
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
}

fn admit(window: &mut Window, maximum: u32, now: Instant) -> AppResult<()> {
    if now.saturating_duration_since(window.started) >= WINDOW {
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

/// 在给定的当前时刻执行窗口重置与计数；纯逻辑便于测试 60 秒窗口的精确边界。
#[cfg(test)]
pub(crate) fn admit_at(window: &Mutex<Window>, maximum: u32, now: Instant) -> AppResult<()> {
    // 临界区只有计数赋值，中毒时数据仍然完整；恢复使用，避免此后每个请求都返回 500。
    let mut window = window.lock().unwrap_or_else(PoisonError::into_inner);
    admit(&mut window, maximum, now)
}

pub(crate) fn admit_bucket<K: Copy + Eq + Hash>(
    buckets: &Mutex<HashMap<K, Window>>,
    capacity: usize,
    key: K,
    maximum: u32,
    now: Instant,
) -> AppResult<()> {
    let mut buckets = buckets.lock().unwrap_or_else(PoisonError::into_inner);
    // 先淘汰过期窗口；若 60 秒内仍被活跃键占满（大量随机 IPv6 或问卷 ID），必须整体清空，
    // 否则映射会突破上限无界增长。清空只重置计数，不影响任何请求的合法性判定。
    if buckets.len() >= capacity {
        buckets.retain(|_, window| now.saturating_duration_since(window.started) < WINDOW);
        if buckets.len() >= capacity {
            buckets.clear();
        }
    }
    let window = buckets.entry(key).or_insert(Window {
        started: now,
        count: 0,
    });
    admit(window, maximum, now)
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
        let now = Instant::now();
        // 只有登录与注册消耗认证配额；退出登录和改密不能被全局限流锁死。
        if path == "/api/auth/login" || path == "/api/auth/register" {
            let salt = state.limits.daily_salt()?;
            let client = client_ip(request, state.config.trust_proxy);
            admit_bucket(
                &state.limits.auth,
                MAX_TRACKED_BUCKETS,
                bucket_key(&salt, client),
                state.config.auth_requests_per_minute,
                now,
            )?;
        } else if *method == Method::POST
            && path.starts_with("/api/public/surveys/")
            && path.ends_with("/responses")
        {
            let salt = state.limits.daily_salt()?;
            let client = client_ip(request, state.config.trust_proxy);
            // 路径参数非法时统一落在一个哨兵问卷桶，后续仍由路径提取器返回 400。
            let survey_id = submission_survey_id(path).unwrap_or_else(Uuid::nil);
            admit_bucket(
                &state.limits.submissions,
                MAX_TRACKED_BUCKETS,
                (bucket_key(&salt, client), survey_id),
                state.config.submissions_per_minute,
                now,
            )?;
        }
    }
    Ok(())
}

fn submission_survey_id(path: &str) -> Option<Uuid> {
    path.strip_prefix("/api/public/surveys/")?
        .strip_suffix("/responses")?
        .parse()
        .ok()
}
