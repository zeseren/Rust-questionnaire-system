//! 错误契约与 HTTP 边界：稳定错误码、内部错误标识、提取器拒绝与分页换算。

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, FromRequest, FromRequestParts},
    http::{HeaderMap, Request, StatusCode, header},
    routing::{get, post},
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    auth::RegisterRequest,
    domain::ValidationError,
    error::AppError,
    http::{ApiJson, ApiPath, ApiQuery, PageBounds, Pagination},
    tests::support::{error_json, send_raw_http, test_state},
};

#[tokio::test]
async fn 业务错误构造器映射稳定状态码与错误体() {
    let 题目 = Uuid::new_v4();
    for (错误, 状态, 码, 消息, 题目编号) in [
        (
            AppError::validation("输入不合法"),
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "输入不合法",
            None,
        ),
        (
            AppError::field(题目, "该题缺失"),
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "该题缺失",
            Some(题目),
        ),
        (
            AppError::conflict("revision_conflict", "草稿已变化"),
            StatusCode::CONFLICT,
            "revision_conflict",
            "草稿已变化",
            None,
        ),
        (
            AppError::not_found(),
            StatusCode::NOT_FOUND,
            "not_found",
            "资源不存在或不可访问",
            None,
        ),
        (
            AppError::unauthorized(),
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "请重新登录",
            None,
        ),
        (
            AppError::forbidden(),
            StatusCode::FORBIDDEN,
            "forbidden",
            "没有执行此操作的权限",
            None,
        ),
    ] {
        let (实际状态, _, body) = error_json(错误).await;
        assert_eq!(实际状态, 状态);
        assert_eq!(body["error"]["code"], 码);
        assert_eq!(body["error"]["message"], 消息);
        match 题目编号 {
            Some(id) => assert_eq!(body["error"]["question_id"], json!(id)),
            None => assert!(body["error"]["question_id"].is_null()),
        }
    }
}

#[tokio::test]
async fn 内部错误生成十六位十六进制标识且不泄露原文() {
    let (status, headers, body) = error_json(AppError::internal("password=秘密 输入原文")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"]["code"], "internal_error");
    assert_eq!(body["error"]["message"], "服务暂时无法完成请求");
    let 标识 = headers
        .get("x-error-id")
        .expect("内部错误必须带检索标识")
        .to_str()
        .expect("标识是可见 ASCII");
    assert_eq!(标识.len(), 16, "标识固定为 8 字节十六进制");
    assert!(标识.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let 响应文本 = body.to_string();
    assert!(!响应文本.contains("秘密"), "响应体不能泄露原始错误");
    assert!(!响应文本.contains("password"), "响应体不能泄露原始错误");
}

#[tokio::test]
async fn 非内部错误不生成检索标识() {
    for 错误 in [
        AppError::validation("字段错误"),
        AppError::not_found(),
        AppError::conflict("already", "已存在"),
    ] {
        let (_, headers, _) = error_json(错误).await;
        assert!(
            headers.get("x-error-id").is_none(),
            "只有内部错误需要故障标识"
        );
    }
}

#[tokio::test]
async fn 外部错误统一收敛为不泄露细节的内部错误() {
    let 题目 = Uuid::new_v4();
    // 三类外部错误都走同一个收敛分支，完整信封已由内部错误用例覆盖。
    let 数据库错误: AppError = sea_orm::DbErr::RecordNotFound("用户 42".to_owned()).into();
    let (状态, 头, _) = error_json(数据库错误).await;
    assert_eq!(状态, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(头.contains_key("x-error-id"), "外部错误也必须带检索标识");

    let 题目错误: AppError = ValidationError {
        question_id: Some(题目),
        message: "答案超出长度".to_owned(),
    }
    .into();
    let (状态, _, body) = error_json(题目错误).await;
    assert_eq!(状态, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["question_id"], json!(题目));

    let 整体错误: AppError = ValidationError {
        question_id: None,
        message: "题面缺少标题".to_owned(),
    }
    .into();
    let (状态, _, body) = error_json(整体错误).await;
    assert_eq!(状态, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["message"], "题面缺少标题");
    assert!(body["error"]["question_id"].is_null());
}

async fn json_提取(内容类型: Option<&str>, body: &str) -> (StatusCode, HeaderMap, Value) {
    let state = test_state();
    let mut builder = Request::builder().method("POST").uri("/api/auth/register");
    if let Some(值) = 内容类型 {
        builder = builder.header(header::CONTENT_TYPE, 值);
    }
    let request = builder
        .body(Body::from(body.to_owned()))
        .expect("测试请求必须可构造");
    let error = ApiJson::<RegisterRequest>::from_request(request, &state)
        .await
        .err()
        .expect("非法 JSON 必须被拒绝");
    error_json(error).await
}

#[tokio::test]
async fn 非法_json_统一映射为四百_invalid_json() {
    for (说明, 内容类型, 原文) in [
        ("语法错误", Some("application/json"), r#"{"username":"#),
        (
            "未知字段",
            Some("application/json"),
            r#"{"username":"alice_01","password":"password123","role":"admin"}"#,
        ),
        (
            "缺少内容类型",
            None,
            r#"{"username":"alice","password":"12345678"}"#,
        ),
    ] {
        let (状态, _, 响应) = json_提取(内容类型, 原文).await;
        assert_eq!(状态, StatusCode::BAD_REQUEST, "{说明}必须返回 400");
        assert_eq!(响应["error"]["code"], "invalid_json", "{说明}");
        assert_eq!(响应["error"]["message"], "请求 JSON 格式或字段类型错误");
    }
}

/// 只用于验证请求体上限的最小处理函数。
async fn 注册回显(ApiJson(请求): ApiJson<RegisterRequest>) -> Json<String> {
    Json(请求.username)
}

#[tokio::test]
async fn 超过请求体上限映射为四百一十三() {
    let router = Router::new()
        .route("/api/auth/register", post(注册回显))
        .layer(DefaultBodyLimit::max(64))
        .with_state(test_state());
    let body = format!(
        r#"{{"username":"{}","password":"12345678"}}"#,
        "a".repeat(200)
    );
    let 请求 = format!(
        "POST /api/auth/register HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let (状态, _, body) = send_raw_http(router, &请求).await;
    assert_eq!(状态, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"]["code"], "body_too_large");
    assert_eq!(body["error"]["message"], "请求体不能超过 1 MiB");
}

#[tokio::test]
async fn 查询参数解析失败映射为四百_invalid_query() {
    let state = test_state();
    let mut parts = Request::builder()
        .uri("/api/surveys?page=2&page_size=50")
        .body(Body::empty())
        .expect("测试请求必须可构造")
        .into_parts()
        .0;
    let ApiQuery(分页) = ApiQuery::<Pagination>::from_request_parts(&mut parts, &state)
        .await
        .expect("合法查询参数必须解析成功");
    assert_eq!(分页.page, Some(2));
    assert_eq!(分页.page_size, Some(50));

    // 非法输入共用同一个 `map_err`，只保留一个负例代表。
    let mut parts = Request::builder()
        .uri("/api/surveys?page=abc")
        .body(Body::empty())
        .expect("测试请求必须可构造")
        .into_parts()
        .0;
    let error = ApiQuery::<Pagination>::from_request_parts(&mut parts, &state)
        .await
        .err()
        .expect("非法查询参数必须被拒绝");
    let (状态, _, body) = error_json(error).await;
    assert_eq!(状态, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_query");
    assert_eq!(body["error"]["message"], "查询参数格式错误");
}

/// 只用于验证路径参数解析的最小处理函数。
async fn 显示路径(ApiPath(id): ApiPath<Uuid>) -> Json<String> {
    Json(id.to_string())
}

#[tokio::test]
async fn 路径参数解析失败映射为四百_invalid_path() {
    let 编号 = Uuid::new_v4();
    let 合法 =
        format!("GET /api/items/{编号} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    let router = Router::new()
        .route("/api/items/{id}", get(显示路径))
        .with_state(test_state());
    let (状态, _, body) = send_raw_http(router, &合法).await;
    assert_eq!(状态, StatusCode::OK);
    assert_eq!(body, json!(编号.to_string()));

    let router = Router::new()
        .route("/api/items/{id}", get(显示路径))
        .with_state(test_state());
    let (状态, _, body) = send_raw_http(
        router,
        "GET /api/items/not-a-uuid HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(状态, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_path");
    assert_eq!(body["error"]["message"], "路径参数格式错误");
}

fn 分页(page: Option<u64>, page_size: Option<u64>) -> Pagination {
    Pagination { page, page_size }
}

#[test]
fn 分页默认值为第一页二十条() {
    assert_eq!(
        分页(None, None).bounds().expect("默认分页合法"),
        PageBounds {
            page: 1,
            page_size: 20,
            offset: 0,
            limit: 20,
        }
    );
}

#[test]
fn 分页边界为一与一百() {
    for (page, size, offset) in [(1, 1, 0), (5, 100, 400), (1, 100, 0)] {
        assert_eq!(
            分页(Some(page), Some(size)).bounds().expect("边界值合法"),
            PageBounds {
                page,
                page_size: size,
                offset,
                limit: size,
            }
        );
    }
}

#[tokio::test]
async fn 非法页码与页大小被拒绝() {
    for (page, size) in [
        (Some(0), Some(20)),
        (Some(1), Some(0)),
        (Some(1), Some(101)),
    ] {
        let error = 分页(page, size).bounds().expect_err("必须拒绝");
        let (状态, _, body) = error_json(error).await;
        assert_eq!(状态, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["code"], "validation_failed");
        assert_eq!(
            body["error"]["message"],
            "page 至少为 1，page_size 必须在 1..=100"
        );
    }
}

#[tokio::test]
async fn 偏移量必须能放进_i64() {
    // (page - 1) * page_size 恰好等于 i64::MAX 的最邻近可行值仍然合法。
    let 边界 = 分页(Some(92_233_720_368_547_759), Some(100))
        .bounds()
        .expect("偏移等于 9223372036854775800 仍可放进 i64");
    assert_eq!(边界.offset, 9_223_372_036_854_775_800);

    for page in [92_233_720_368_547_760, u64::MAX] {
        let error = 分页(Some(page), Some(100))
            .bounds()
            .expect_err("偏移超出 i64 必须拒绝");
        let (状态, _, body) = error_json(error).await;
        assert_eq!(状态, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["message"], "页码过大");
    }
}

#[test]
fn 分页字段兼容数字与十进制字符串并拒绝非纯十进制() {
    let 期望 = PageBounds {
        page: 2,
        page_size: 50,
        offset: 50,
        limit: 50,
    };
    for 输入 in [
        r#"{"page":2,"page_size":50}"#,
        r#"{"page":"2","page_size":"50"}"#,
    ] {
        let 分页值 =
            serde_json::from_str::<Pagination>(输入).expect("JSON 数字与十进制字符串必须兼容");
        assert_eq!(分页值.bounds().expect("数字分页合法"), 期望, "{输入}");
    }

    let 缺省 = serde_json::from_str::<Pagination>("{}").expect("缺省字段合法");
    assert_eq!(缺省.page, None);
    assert_eq!(缺省.page_size, None);

    // 有符号、空串、小数、非数字共用同一条拒绝分支，只保留一个代表值与一个溢出值。
    for 输入 in [r#"{"page":"-1"}"#, r#"{"page":"18446744073709551616"}"#] {
        let error = serde_json::from_str::<Pagination>(输入).expect_err("非纯十进制字符串必须拒绝");
        assert!(
            error.to_string().contains("非负整数"),
            "拒绝理由必须指向非负整数：{输入} -> {error}"
        );
    }
}
