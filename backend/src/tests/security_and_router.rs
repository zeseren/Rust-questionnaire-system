//! 请求闸门与路由装配：CSRF、固定窗口限流桶、开放文档、fallback 与统一响应头。

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::http::{StatusCode, header};
use uuid::Uuid;

use crate::{
    AppState,
    security::{self, Window, admit_at},
    tests::support::{error_json, raw_request, send_raw_http, test_config, test_state},
};

const 同源: [(&str, &str); 2] = [
    ("origin", "http://localhost:3000"),
    ("x-csrf-protection", "1"),
];

/// 构造最小合法的原始 HTTP 请求；空体不发送 `Content-Length`。
fn 报文(方法: &str, 路径: &str, 体: &str) -> String {
    let mut 报文 = format!("{方法} {路径} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    if !体.is_empty() {
        报文.push_str(&format!("Content-Length: {}\r\n", 体.len()));
    }
    报文.push_str("\r\n");
    报文.push_str(体);
    报文
}

#[tokio::test]
async fn 安全方法免于_csrf_与限流() {
    let state = test_state();
    // 连续 9 次安全请求远超认证配额，仍必须全部通过。
    for _ in 0..3 {
        for 方法 in ["GET", "HEAD", "OPTIONS"] {
            assert!(
                security::check(&state, &raw_request(方法, "/api/auth/login", &[])).is_ok(),
                "{方法} 不接受浏览器表单跨站提交，必须免检"
            );
        }
    }
    // 安全方法不占用配额：认证桶仍可完整用满 2 次，第 3 次才限流。
    for _ in 0..2 {
        assert!(
            security::check(&state, &raw_request("POST", "/api/auth/login", &同源)).is_ok(),
            "安全请求之后认证配额必须仍然完整可用"
        );
    }
    let error = security::check(&state, &raw_request("POST", "/api/auth/login", &同源))
        .expect_err("第 3 次必须限流");
    assert_eq!(error_json(error).await.0, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn 写方法必须携带同源来源与防护头() {
    let state = test_state();
    // 4 种拒绝原因都在 POST 上覆盖，其余写方法只验证同样受保护且能通过。
    for (说明, 头) in [
        ("缺少 Origin", &[("x-csrf-protection", "1")][..]),
        (
            "Origin 不同源",
            &[
                ("origin", "http://localhost:4000"),
                ("x-csrf-protection", "1"),
            ][..],
        ),
        ("缺少防护头", &[("origin", "http://localhost:3000")][..]),
        (
            "防护头不是 1",
            &[
                ("origin", "http://localhost:3000"),
                ("x-csrf-protection", "true"),
            ][..],
        ),
    ] {
        let error = security::check(&state, &raw_request("POST", "/api/surveys", 头))
            .expect_err("非法写请求必须拒绝");
        let (状态, _, body) = error_json(error).await;
        assert_eq!(状态, StatusCode::FORBIDDEN, "POST：{说明}");
        assert_eq!(body["error"]["code"], "csrf_rejected", "POST：{说明}");
    }
    let error = security::check(
        &state,
        &raw_request("POST", "/api/surveys", &[("x-csrf-protection", "1")]),
    )
    .expect_err("缺少 Origin 必须拒绝");
    let (_, _, body) = error_json(error).await;
    assert_eq!(
        body["error"]["message"],
        "写请求必须来自配置的同源站点并携带 X-CSRF-Protection: 1"
    );

    for 方法 in ["PUT", "PATCH", "DELETE"] {
        assert!(
            security::check(&state, &raw_request(方法, "/api/surveys", &同源)).is_ok(),
            "{方法} 同源请求必须通过"
        );
        let error = security::check(
            &state,
            &raw_request(方法, "/api/surveys", &[("origin", "http://localhost:3000")]),
        )
        .expect_err("缺少防护头的写请求必须拒绝");
        assert_eq!(
            error_json(error).await.0,
            StatusCode::FORBIDDEN,
            "{方法} 必须同样受 CSRF 保护"
        );
    }
}

#[tokio::test]
async fn csrf_失败不占用限流配额() {
    let state = test_state();
    let error = security::check(&state, &raw_request("POST", "/api/auth/login", &[]))
        .expect_err("缺少来源头的写请求必须被 CSRF 拒绝");
    let (状态, _, body) = error_json(error).await;
    assert_eq!(状态, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "csrf_rejected");

    // 被拒绝的请求不占配额：完整 2 次仍然通过，第 3 次才限流。
    for _ in 0..2 {
        assert!(
            security::check(&state, &raw_request("POST", "/api/auth/login", &同源)).is_ok(),
            "上限内的同源请求必须通过"
        );
    }
    let error = security::check(&state, &raw_request("POST", "/api/auth/login", &同源))
        .expect_err("第 3 次必须限流");
    let (状态, _, body) = error_json(error).await;
    assert_eq!(状态, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limited");
    assert_eq!(body["error"]["message"], "请求过于频繁，请稍后重试");
}

#[tokio::test]
async fn 认证与提交使用互不干扰的限流桶() {
    let state = test_state();
    let 提交路径 = format!("/api/public/surveys/{}/responses", Uuid::new_v4());
    for _ in 0..2 {
        assert!(security::check(&state, &raw_request("POST", "/api/auth/login", &同源)).is_ok());
    }
    for _ in 0..3 {
        assert!(
            security::check(&state, &raw_request("POST", &提交路径, &同源)).is_ok(),
            "提交桶上限为 3，不受认证桶用尽影响"
        );
    }
    let 认证超限 = security::check(&state, &raw_request("POST", "/api/auth/login", &同源))
        .expect_err("认证桶第 3 次必须限流");
    let 提交超限 = security::check(&state, &raw_request("POST", &提交路径, &同源))
        .expect_err("提交桶第 4 次必须限流");
    for 错误 in [认证超限, 提交超限] {
        assert_eq!(
            error_json(错误).await.0,
            StatusCode::TOO_MANY_REQUESTS,
            "两个桶各自独立计数"
        );
    }
    // 其余认证入口共用同一个桶：桶满后任一入口都必须限流。
    let error = security::check(&state, &raw_request("POST", "/api/auth/register", &同源))
        .expect_err("认证桶满后所有认证入口都必须限流");
    assert_eq!(error_json(error).await.0, StatusCode::TOO_MANY_REQUESTS);
    // 退出登录与改密不消耗认证配额：即使桶已满也不能被锁死，否则用户无法自救。
    for (方法, 路径) in [("POST", "/api/auth/logout"), ("PUT", "/api/auth/password")] {
        assert!(
            security::check(&state, &raw_request(方法, 路径, &同源)).is_ok(),
            "{路径} 不受认证限流影响"
        );
    }
}

/// 信任代理时按转发头区分客户端；认证桶对每个 IP 独立计数。
#[tokio::test]
async fn 信任代理时按转发头分桶() {
    let mut config = test_config();
    config.trust_proxy = true;
    let state = AppState {
        config: Arc::new(config),
        ..test_state()
    };
    let 请求 = |转发: &str| {
        raw_request(
            "POST",
            "/api/auth/login",
            &[
                ("origin", "http://localhost:3000"),
                ("x-csrf-protection", "1"),
                ("x-forwarded-for", 转发),
            ],
        )
    };

    for _ in 0..2 {
        security::check(&state, &请求("203.0.113.7")).expect("同一客户端在配额内必须通过");
    }
    let error =
        security::check(&state, &请求("203.0.113.7")).expect_err("同一客户端超出配额必须限流");
    assert_eq!(error_json(error).await.0, StatusCode::TOO_MANY_REQUESTS);
    security::check(&state, &请求("203.0.113.8")).expect("不同客户端必须使用各自的分桶");
    security::check(
        &state,
        &raw_request(
            "POST",
            "/api/auth/login",
            &[
                ("origin", "http://localhost:3000"),
                ("x-csrf-protection", "1"),
                ("x-real-ip", "198.51.100.7"),
            ],
        ),
    )
    .expect("缺少 X-Forwarded-For 时回退到 X-Real-IP");
    // 首个非空且合法的转发地址生效：前导空白与非法值都被跳过。
    security::check(&state, &请求("not-an-ip, 203.0.113.9")).expect("首个有效转发地址决定分桶");
    security::check(&state, &请求("not-an-ip, 203.0.113.9")).expect("同一有效转发地址共享分桶");
    let error = security::check(&state, &请求("not-an-ip, 203.0.113.9"))
        .expect_err("同一有效转发地址的第三次请求必须限流");
    assert_eq!(error_json(error).await.0, StatusCode::TOO_MANY_REQUESTS);
}

/// 未信任代理时忽略转发头，所有请求落在一个哨兵桶，客户端无法伪造 IP 绕过。
#[tokio::test]
async fn 未信任代理时忽略转发头() {
    let state = test_state();
    let 请求 = |转发: &str| {
        raw_request(
            "POST",
            "/api/auth/login",
            &[
                ("origin", "http://localhost:3000"),
                ("x-csrf-protection", "1"),
                ("x-forwarded-for", 转发),
            ],
        )
    };
    security::check(&state, &请求("203.0.113.1")).expect("第一次请求在配额内");
    security::check(&state, &请求("203.0.113.2")).expect("伪造不同 IP 不产生新桶");
    let error =
        security::check(&state, &请求("203.0.113.3")).expect_err("共用哨兵桶的第 3 次请求必须限流");
    assert_eq!(error_json(error).await.0, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn 提交桶按问卷独立计数() {
    let state = test_state();
    let 路径 = |id: Uuid| format!("/api/public/surveys/{id}/responses");
    let 第一份 = 路径(Uuid::new_v4());
    let 第二份 = 路径(Uuid::new_v4());
    for _ in 0..3 {
        security::check(&state, &raw_request("POST", &第一份, &同源)).expect("第一份问卷配额内");
    }
    for _ in 0..3 {
        security::check(&state, &raw_request("POST", &第二份, &同源))
            .expect("第二份问卷拥有独立配额");
    }
    assert_eq!(
        error_json(
            security::check(&state, &raw_request("POST", &第一份, &同源))
                .expect_err("第一份问卷超出配额")
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[test]
fn 普通写路径不进入任何限流桶() {
    let state = test_state();
    // 4 轮已超过提交桶上限 3：普通写路径若被计数，这里必然出现 429。
    for _ in 0..4 {
        assert!(security::check(&state, &raw_request("POST", "/api/surveys", &同源)).is_ok());
        assert!(
            security::check(
                &state,
                &raw_request("PUT", "/api/surveys/anything/draft", &同源)
            )
            .is_ok()
        );
    }
}

#[tokio::test]
async fn 限流窗口满六十秒后重置() {
    let 基准 = Instant::now();
    let window = Mutex::new(Window {
        started: 基准,
        count: 1,
    });
    let 未到重置 =
        admit_at(&window, 1, 基准 + Duration::from_secs(59)).expect_err("未满 60 秒必须继续限流");
    let (状态, _, body) = error_json(未到重置).await;
    assert_eq!(状态, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limited");

    admit_at(&window, 1, 基准 + Duration::from_secs(60)).expect("满 60 秒必须重置并放行本次请求");
    let 再次超限 = admit_at(&window, 1, 基准 + Duration::from_secs(61))
        .expect_err("新窗口内的第 2 次必须限流");
    assert_eq!(error_json(再次超限).await.0, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn 活跃桶达到上限且无可淘汰窗口时整体清空() {
    let 基准 = Instant::now();
    let buckets = Mutex::new(std::collections::HashMap::new());
    // 三个键都在同一窗口内活跃，`retain` 一条也删不掉。
    for key in 0..3_u32 {
        security::admit_bucket(&buckets, 3, key, 10, 基准).expect("常规请求必须放行");
    }
    assert_eq!(buckets.lock().expect("加锁成功").len(), 3);
    security::admit_bucket(&buckets, 3, 99, 10, 基准).expect("超限后仍必须放行本次请求");
    let 剩余 = buckets.lock().expect("加锁成功").len();
    assert_eq!(
        剩余, 1,
        "无可淘汰窗口时必须整体清空，仅保留当前请求桶，实际 {剩余}"
    );
}

#[tokio::test]
async fn 互斥锁中毒后限流仍可恢复() {
    let state = test_state();
    let 中毒 = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = state.limits.auth.lock().expect("首次加锁成功");
        panic!("模拟持锁线程崩溃");
    }));
    assert!(中毒.is_err(), "测试必须真的制造中毒状态");
    assert!(state.limits.auth.is_poisoned());

    for _ in 0..2 {
        assert!(
            security::check(&state, &raw_request("POST", "/api/auth/login", &同源)).is_ok(),
            "中毒后必须继续用仍完整的计数工作"
        );
    }
    let error = security::check(&state, &raw_request("POST", "/api/auth/login", &同源))
        .expect_err("恢复后的计数仍然是同一个窗口");
    let (状态, _, body) = error_json(error).await;
    assert_eq!(状态, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limited");
}

fn 路由() -> axum::Router {
    crate::router(test_state())
}

#[tokio::test]
async fn 开放文档包含完整路由表与_cookie_认证方案() {
    let (状态, _, 文档) = send_raw_http(路由(), &报文("GET", "/api/openapi.json", "")).await;
    assert_eq!(状态, StatusCode::OK);
    assert_eq!(文档["info"]["title"], "问卷调查系统 API");
    let paths = 文档["paths"].as_object().expect("文档必须包含路径表");
    for 路径 in [
        "/health",
        "/api/auth/register",
        "/api/auth/login",
        "/api/auth/logout",
        "/api/auth/me",
        "/api/auth/password",
        "/api/surveys",
        "/api/surveys/counts",
        "/api/surveys/{id}",
        "/api/surveys/{id}/draft",
        "/api/surveys/{id}/preview",
        "/api/surveys/{id}/copy",
        "/api/surveys/{id}/publish",
        "/api/surveys/{id}/collection",
        "/api/surveys/{id}/close",
        "/api/surveys/{id}/resume",
        "/api/surveys/{id}/archive",
        "/api/surveys/{id}/versions",
        "/api/public/surveys/{id}",
        "/api/public/surveys/{id}/meta",
        "/api/public/surveys/{id}/responses",
        "/api/surveys/{id}/responses",
        "/api/surveys/{id}/responses/{receipt_id}",
        "/api/surveys/{id}/questions/{question_id}/responses",
        "/api/surveys/{id}/statistics",
        "/api/surveys/{id}/export.csv",
        "/api/admin/users",
        "/api/admin/users/{id}",
        "/api/admin/surveys",
        "/api/admin/surveys/{id}",
        "/api/admin/audits",
    ] {
        assert!(paths.contains_key(路径), "开放文档缺少路由：{路径}");
    }
    // 抽样确认路径下声明的 HTTP 方法，防止文档与路由装配漂移。
    for (路径, 方法) in [
        ("/api/auth/login", "post"),
        ("/api/auth/register", "post"),
        ("/api/surveys", "get"),
        ("/api/surveys", "post"),
        ("/api/surveys/{id}", "get"),
        ("/api/surveys/{id}", "delete"),
        ("/api/surveys/{id}/collection", "patch"),
        ("/api/surveys/{id}/export.csv", "get"),
        ("/api/admin/users/{id}", "patch"),
        ("/health", "get"),
    ] {
        let 项 = paths
            .get(路径)
            .unwrap_or_else(|| panic!("开放文档缺少路由：{路径}"));
        assert!(项.get(方法).is_some(), "{路径} 必须声明 {方法} 操作");
    }
    let 方案 = &文档["components"]["securitySchemes"]["cookieAuth"];
    assert_eq!(方案["name"], "session", "认证方案必须指向 session Cookie");
    assert_eq!(方案["in"], "cookie");
}

#[tokio::test]
async fn 未知路径返回四百零四_not_found() {
    let (状态, _, body) = send_raw_http(路由(), &报文("GET", "/api/does-not-exist", "")).await;
    assert_eq!(状态, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn 已知路径的错误方法返回四百零五() {
    let (状态, _, body) = send_raw_http(路由(), &报文("GET", "/api/auth/login", "")).await;
    assert_eq!(状态, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body["error"]["code"], "method_not_allowed");
    assert_eq!(body["error"]["message"], "不支持此请求方法");
}

#[tokio::test]
async fn 数据库未连接时健康检查返回五百零三() {
    let (状态, _, body) = send_raw_http(路由(), &报文("GET", "/health", "")).await;
    assert_eq!(状态, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "not_ready");
    assert_eq!(body["error"]["message"], "数据库尚未就绪");
}

#[tokio::test]
async fn 所有响应都带统一安全响应头且不开放_cors() {
    let 匿名登录 = 报文("POST", "/api/auth/login", "{}");
    for (说明, 请求) in [
        ("开放文档", 报文("GET", "/api/openapi.json", "")),
        ("跨站写请求", 匿名登录),
        ("未知路径", 报文("GET", "/api/nope", "")),
        ("方法不匹配", 报文("GET", "/api/auth/login", "")),
        ("健康检查", 报文("GET", "/health", "")),
    ] {
        let (_, 头, _) = send_raw_http(路由(), &请求).await;
        assert_eq!(头[header::CACHE_CONTROL], "no-store", "{说明}必须禁止缓存");
        assert_eq!(
            头["x-content-type-options"], "nosniff",
            "{说明}必须禁止类型嗅探"
        );
        assert!(
            !头.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            "{说明}不能开放 CORS"
        );
    }
}
