//! 账号凭据与管理员治理：用户名/密码边界、Argon2id、会话秘密与治理输入校验。

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::{
    admin::{SurveyItem, SurveyRow, reason, require_admin},
    auth::{
        AuthUser, RegisterRequest, UserProfile, hash_password, hash_secret, hash_secret_bytes,
        invalid_credentials, new_session_token, normalize_username, session_cookie, session_token,
        validate_password, verify_password,
    },
    entities::{SurveyStatus, UserRole},
    tests::support::{error_json, user_model},
};

/// 64 位小写十六进制，等价于一次合法会话秘密。
fn 会话秘密() -> String {
    "0123456789abcdef".repeat(4)
}

fn cookie_头(值: &[&str]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for 项 in 值 {
        headers.append(
            header::COOKIE,
            HeaderValue::from_str(项).expect("测试 Cookie 必须是可见 ASCII"),
        );
    }
    headers
}

#[tokio::test]
async fn 用户名必须为三到三十二个_ascii_字母数字下划线() {
    assert_eq!(
        normalize_username("abc".to_owned()).expect("最短用户名合法"),
        "abc"
    );
    assert_eq!(
        normalize_username("ABC_123".to_owned()).expect("大写折叠为小写"),
        "abc_123",
        "保存前统一小写，避免大小写重复注册"
    );
    assert_eq!(
        normalize_username("a".repeat(32))
            .expect("最长用户名合法")
            .len(),
        32
    );

    for 非法 in [
        "ab".to_owned(),
        "a".repeat(33),
        "user-name".to_owned(),
        "user name".to_owned(),
        "用户abc".to_owned(),
    ] {
        let error = normalize_username(非法.clone()).expect_err("必须拒绝");
        let (状态, _, body) = error_json(error).await;
        assert_eq!(状态, StatusCode::UNPROCESSABLE_ENTITY, "非法用户名：{非法}");
        assert_eq!(body["error"]["code"], "validation_failed");
        assert_eq!(
            body["error"]["message"],
            "用户名须为 3—32 个 ASCII 字母、数字或下划线"
        );
    }
}

#[tokio::test]
async fn 密码按字符数限制在八到一百二十八之间() {
    validate_password("12345678").expect("最短密码合法");
    validate_password(&"x".repeat(128)).expect("最长密码合法");
    validate_password(&"😀".repeat(8)).expect("8 个 Emoji 按字符计数合法");

    for 非法 in ["1".repeat(7), "x".repeat(129), "😀".repeat(129)] {
        let error = validate_password(&非法).expect_err("必须拒绝");
        let (状态, _, body) = error_json(error).await;
        assert_eq!(状态, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["code"], "validation_failed");
        assert_eq!(body["error"]["message"], "密码长度须为 8—128 个字符");
    }
}

#[tokio::test]
async fn 登录失败不区分具体原因() {
    let (状态, _, body) = error_json(invalid_credentials()).await;
    assert_eq!(状态, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "invalid_credentials");
    assert_eq!(
        body["error"]["message"], "用户名或密码错误",
        "不得泄露账号是否存在"
    );
}

#[tokio::test]
async fn argon2id_哈希随机加盐并可验证() {
    let 密码 = "正确的马电池订书钉".to_owned();
    let 第一份 = hash_password(密码.clone()).await.expect("哈希成功");
    let 第二份 = hash_password(密码.clone()).await.expect("哈希成功");
    assert!(第一份.starts_with("$argon2id$"), "必须使用 Argon2id");
    assert_ne!(第一份, 第二份, "随机盐必须让同一密码得到不同摘要");
    assert!(
        verify_password(密码.clone(), 第一份.clone())
            .await
            .expect("验证不报错"),
        "正确密码必须通过"
    );
    assert!(
        !verify_password("错误密码123".to_owned(), 第一份)
            .await
            .expect("密码不匹配不是内部错误"),
        "错误密码返回 Ok(false) 而不是错误"
    );
}

#[tokio::test]
async fn 损坏的密码摘要被识别为内部错误() {
    let error = verify_password("任意密码".to_owned(), "不是 PHC 字符串".to_owned())
        .await
        .expect_err("损坏摘要必须报错");
    let (状态, 头, body) = error_json(error).await;
    assert_eq!(状态, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"]["code"], "internal_error");
    assert_eq!(body["error"]["message"], "服务暂时无法完成请求");
    assert!(头.get("x-error-id").is_some(), "内部错误必须可检索");
    assert!(!body.to_string().contains("PHC"), "不能泄露摘要原文");
}

#[test]
fn 会话秘密为六十四位小写十六进制且不可预测() {
    let 第一份 = new_session_token().expect("随机源可用");
    let 第二份 = new_session_token().expect("随机源可用");
    assert_eq!(第一份.len(), 64);
    assert!(
        第一份
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "只允许小写十六进制"
    );
    assert_ne!(第一份, 第二份, "每次登录必须生成新的秘密");
}

#[test]
fn 秘密摘要为确定性的_sha256() {
    let 摘要 = hash_secret("abc");
    assert_eq!(
        摘要, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "必须是标准 SHA-256 十六进制"
    );
    assert_eq!(hash_secret("abc"), 摘要, "同一输入必须得到同一摘要");
    assert_eq!(hash_secret_bytes(b"abc"), 摘要);
    assert_ne!(hash_secret("abc"), hash_secret("abd"));
}

#[test]
fn 会话_cookie_只从唯一且格式正确的_cookie_中读取() {
    let 秘密 = 会话秘密();
    let 单个 = format!("session={秘密}");
    assert_eq!(
        session_token(&cookie_头(&[单个.as_str()])),
        Some(秘密.as_str()),
        "单一 session Cookie 可用"
    );

    let 共存 = format!("other=1; session={秘密}; trailing=2");
    assert_eq!(
        session_token(&cookie_头(&[共存.as_str()])),
        Some(秘密.as_str()),
        "与其他 Cookie 共存不影响识别"
    );
    assert_eq!(
        session_token(&cookie_头(&["other=1", 单个.as_str()])),
        Some(秘密.as_str()),
        "session 可以出现在另一条 Cookie 头"
    );

    let 另一个 = "f".repeat(64);
    let 同头重复 = format!("session={秘密}; session={另一个}");
    let 第一头 = format!("session={秘密}");
    let 第二头 = format!("session={另一个}");
    let 长度不足 = format!("session={}", "a".repeat(63));
    let 长度超出 = format!("session={}", "a".repeat(65));
    let 非十六进制 = format!("session={}", "g".repeat(64));
    for (说明, 头) in [
        ("同一头重复同名", cookie_头(&[同头重复.as_str()])),
        (
            "跨头重复同名",
            cookie_头(&[第一头.as_str(), 第二头.as_str()]),
        ),
        ("长度不足", cookie_头(&[长度不足.as_str()])),
        ("长度超出", cookie_头(&[长度超出.as_str()])),
        ("非十六进制", cookie_头(&[非十六进制.as_str()])),
        ("没有会话 Cookie", cookie_头(&["other=1"])),
    ] {
        assert_eq!(session_token(&头), None, "{说明}必须视为未登录");
    }
}

#[test]
fn 会话_cookie_固定安全属性() {
    let 秘密 = 会话秘密();
    let 明文 = session_cookie(&秘密, 3_600, false)
        .expect("Cookie 头合法")
        .to_str()
        .expect("Cookie 是可见 ASCII")
        .to_owned();
    for 期望 in [
        format!("session={秘密}"),
        "HttpOnly".to_owned(),
        "SameSite=Lax".to_owned(),
        "Path=/".to_owned(),
        "Max-Age=3600".to_owned(),
    ] {
        assert!(明文.contains(&期望), "缺少属性 {期望}：{明文}");
    }
    assert!(!明文.contains("Secure"), "本机 HTTP 开发不发送 Secure");
    assert!(!明文.contains("Domain="), "不限定域名");

    let 清除 = session_cookie("", 0, true)
        .expect("Cookie 头合法")
        .to_str()
        .expect("Cookie 是可见 ASCII")
        .to_owned();
    assert!(清除.contains("Max-Age=0"), "退出必须立即过期：{清除}");
    assert!(清除.contains("Secure"), "HTTPS 部署必须带 Secure：{清除}");
}

#[test]
fn 注册请求拒绝额外字段() {
    let 请求 = serde_json::json!({
        "username": "alice",
        "password": "12345678",
        "role": "admin",
    });
    let error = serde_json::from_value::<RegisterRequest>(请求)
        .err()
        .expect("客户端不能自带角色");
    assert!(
        error.to_string().contains("unknown field"),
        "必须明确拒绝未知字段：{error}"
    );
}

#[test]
fn 账号资料只暴露身份与角色() {
    let model = user_model();
    let 资料 = UserProfile::from(model.clone());
    assert_eq!(资料.id, model.id);
    assert_eq!(资料.username, "alice");
    assert_eq!(资料.role, UserRole::User);
    let json = serde_json::to_value(&资料).expect("资料可序列化");
    assert_eq!(
        json.as_object().expect("资料是对象").len(),
        3,
        "只允许 id、username、role 三个字段"
    );
    assert!(json.get("password_hash").is_none());
    assert!(!json.to_string().contains("argon2"), "序列化结果不能带摘要");
}

#[tokio::test]
async fn 治理接口只允许管理员() {
    let 管理员 = AuthUser {
        id: Uuid::new_v4(),
        username: "root".to_owned(),
        role: UserRole::Admin,
    };
    require_admin(&管理员).expect("管理员必须通过");
    let 普通用户 = AuthUser {
        role: UserRole::User,
        ..管理员
    };
    let error = require_admin(&普通用户).expect_err("普通用户必须拒绝");
    let (状态, _, body) = error_json(error).await;
    assert_eq!(状态, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "forbidden");
    assert_eq!(body["error"]["message"], "没有执行此操作的权限");
}

#[tokio::test]
async fn 治理原因必须是一到五百个可显示字符() {
    for 合法 in [
        "x".to_owned(),
        "好".to_owned(),
        "a".repeat(500),
        "说".repeat(500),
        " 两边空白但非空 ".to_owned(),
    ] {
        reason(&合法).expect("合法原因必须通过");
    }

    for (非法, 期望) in [
        ("包含\0空字符".to_owned(), "治理原因不能包含 NUL 字符"),
        (String::new(), "治理原因须为 1..=500 个字符且不能全为空白"),
        (
            "   \t\n ".to_owned(),
            "治理原因须为 1..=500 个字符且不能全为空白",
        ),
        (
            "多".repeat(501),
            "治理原因须为 1..=500 个字符且不能全为空白",
        ),
    ] {
        let error = reason(&非法).expect_err("必须拒绝");
        let (状态, _, body) = error_json(error).await;
        assert_eq!(状态, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["code"], "validation_failed");
        assert_eq!(body["error"]["message"], 期望);
    }
}

#[test]
fn 治理列表按到期时间物化状态并保留字段() {
    let now = Utc::now();
    let 到期 = now - Duration::seconds(1);
    let 编号 = Uuid::new_v4();
    let 所有者 = Uuid::new_v4();
    let 行 = |status: SurveyStatus, deadline| SurveyRow {
        id: 编号,
        owner_id: 所有者,
        title: "治理标题".to_owned(),
        status,
        taken_down: true,
        deadline,
        current_version_id: Some(Uuid::new_v4()),
    };

    let 已到期 = SurveyItem::from_row(行(SurveyStatus::Collecting, Some(到期)), now);
    assert_eq!(已到期.status, SurveyStatus::Closed, "到期即视为已关闭");
    assert_eq!(已到期.id, 编号);
    assert_eq!(已到期.owner_id, 所有者);
    assert_eq!(已到期.title, "治理标题");
    assert!(已到期.taken_down);
    assert_eq!(已到期.deadline, Some(到期));
    assert!(已到期.current_version_id.is_some());

    assert_eq!(
        SurveyItem::from_row(行(SurveyStatus::Collecting, None), now).status,
        SurveyStatus::Collecting,
        "没有截止时间保持回收中"
    );
    assert_eq!(
        SurveyItem::from_row(行(SurveyStatus::Archived, Some(到期)), now).status,
        SurveyStatus::Archived,
        "归档状态不受截止时间影响"
    );
}
