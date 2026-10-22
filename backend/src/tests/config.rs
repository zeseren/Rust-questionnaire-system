//! 启动配置的全分支：默认值、URL 形式、HTTP 与 Cookie 的矛盾、数值范围与不可解码的变量。

use std::{collections::BTreeMap, env};

use crate::config::Config;

const 数据库: &str = "postgres://user:pass@127.0.0.1:5432/questionnaire";

/// 用查找表替代进程环境：重复键以最后一次出现为准，未出现的键返回 `NotPresent`。
fn 查找(条目: &[(&str, &str)]) -> impl Fn(&str) -> Result<String, env::VarError> {
    let 表: BTreeMap<String, String> = 条目
        .iter()
        .map(|(名称, 值)| ((*名称).to_owned(), (*值).to_owned()))
        .collect();
    move |名称| 表.get(名称).cloned().ok_or(env::VarError::NotPresent)
}

fn 配置(附加: &[(&str, &str)]) -> anyhow::Result<Config> {
    let mut 条目 = vec![("DATABASE_URL", 数据库)];
    条目.extend_from_slice(附加);
    Config::from_lookup(查找(&条目))
}

fn 拒绝原因(附加: &[(&str, &str)]) -> String {
    配置(附加).err().expect("该配置必须被拒绝").to_string()
}

#[test]
fn 只设置数据库时采用开发默认值() {
    let config = 配置(&[]).expect("仅 DATABASE_URL 即可启动");
    assert_eq!(config.database_url, 数据库);
    assert_eq!(
        config.listen,
        "127.0.0.1:3000".parse().expect("默认地址合法")
    );
    assert_eq!(config.public_origin, "http://localhost:3000");
    assert!(!config.cookie_secure, "本机开发默认允许非 Secure Cookie");
    assert_eq!(config.session_ttl_seconds, 86_400);
    assert_eq!(config.auth_requests_per_minute, 60);
    assert_eq!(config.submissions_per_minute, 600);
}

#[test]
fn 缺少数据库地址时给出配置项名称() {
    let error = Config::from_lookup(查找(&[]))
        .err()
        .expect("必须要求 DATABASE_URL");
    assert_eq!(error.to_string(), "请设置 DATABASE_URL");
}

#[test]
fn 数据库连接串必须是合法的_postgresql_url() {
    assert_eq!(
        配置(&[("DATABASE_URL", "这不是 URL")])
            .err()
            .unwrap()
            .to_string(),
        "DATABASE_URL 格式错误"
    );
    assert_eq!(
        配置(&[("DATABASE_URL", "mysql://user:pass@127.0.0.1:3306/db")])
            .err()
            .unwrap()
            .to_string(),
        "DATABASE_URL 必须使用 PostgreSQL"
    );
    assert_eq!(
        配置(&[("DATABASE_URL", "postgresql://user:pass@127.0.0.1:5432/db")])
            .expect("postgresql 协议同样合法")
            .database_url,
        "postgresql://user:pass@127.0.0.1:5432/db"
    );
}

#[test]
fn 公开来源必须是裸_origin() {
    for 来源 in [
        "http://localhost:3000/app",
        "http://localhost:3000/?x=1",
        "http://localhost:3000/",
        "ftp://localhost:3000",
    ] {
        assert_eq!(
            拒绝原因(&[("PUBLIC_ORIGIN", 来源)]),
            "PUBLIC_ORIGIN 必须是无路径的 http(s) origin，例如 http://localhost:3000",
            "非裸 origin 必须拒绝：{来源}"
        );
    }
    assert_eq!(
        拒绝原因(&[("PUBLIC_ORIGIN", "http://")]),
        "PUBLIC_ORIGIN 格式错误",
        "缺少 host 时连解析都不通过"
    );
}

#[test]
fn 非本机的_http_来源被拒绝而本机地址允许() {
    assert_eq!(
        拒绝原因(&[("PUBLIC_ORIGIN", "http://example.com")]),
        "非本机部署必须使用 HTTPS"
    );
    for 来源 in [
        "http://localhost:8080",
        "http://127.0.0.1:8080",
        "http://[::1]:8080",
    ] {
        assert_eq!(
            配置(&[("PUBLIC_ORIGIN", 来源)])
                .expect("本机 http 开发地址允许")
                .public_origin,
            来源
        );
    }
}

#[test]
fn https_与_cookie_安全标志必须一致() {
    let 安全 = 配置(&[
        ("PUBLIC_ORIGIN", "https://survey.example.com"),
        ("COOKIE_SECURE", "true"),
    ])
    .expect("HTTPS 加 Secure Cookie 合法");
    assert!(安全.cookie_secure);
    assert_eq!(安全.public_origin, "https://survey.example.com");

    assert_eq!(
        拒绝原因(&[("PUBLIC_ORIGIN", "https://survey.example.com")]),
        "HTTPS 部署要求 COOKIE_SECURE=true"
    );
    assert_eq!(
        拒绝原因(&[("COOKIE_SECURE", "true")]),
        "HTTP 无法使用 Secure Cookie；请使用 HTTPS 或本机开发设置"
    );
    assert_eq!(
        拒绝原因(&[("COOKIE_SECURE", "yes")]),
        "COOKIE_SECURE 必须为 true 或 false"
    );
}

#[test]
fn 监听地址必须是_ip_与端口() {
    assert_eq!(
        拒绝原因(&[("LISTEN_ADDR", "localhost:3000")]),
        "LISTEN_ADDR 必须是 IP:端口"
    );
    assert_eq!(
        拒绝原因(&[("LISTEN_ADDR", "127.0.0.1")]),
        "LISTEN_ADDR 必须是 IP:端口"
    );
    assert_eq!(
        配置(&[("LISTEN_ADDR", "0.0.0.0:8080")])
            .expect("显式 IPv4 地址合法")
            .listen,
        "0.0.0.0:8080".parse().expect("地址合法")
    );
}

#[test]
fn 会话有效期只接受一秒到三十天的闭区间() {
    for (值, 期望) in [("1", 1_i64), ("2592000", 2_592_000)] {
        assert_eq!(
            配置(&[("SESSION_TTL_SECONDS", 值)])
                .expect("区间边界合法")
                .session_ttl_seconds,
            期望
        );
    }
    for 值 in ["0", "2592001", "-1"] {
        assert_eq!(
            拒绝原因(&[("SESSION_TTL_SECONDS", 值)]),
            "会话有效期须为 1..=2592000 秒，限流次数必须大于零",
            "越界值必须拒绝：{值}"
        );
    }
    assert_eq!(
        拒绝原因(&[("SESSION_TTL_SECONDS", "一小时")]),
        "SESSION_TTL_SECONDS 必须是整数秒"
    );
}

#[test]
fn 两个限流参数必须为正整数() {
    assert_eq!(
        配置(&[
            ("AUTH_REQUESTS_PER_MINUTE", "1"),
            ("SUBMISSIONS_PER_MINUTE", "1")
        ])
        .expect("最小正整数合法")
        .submissions_per_minute,
        1
    );
    for 名称 in ["AUTH_REQUESTS_PER_MINUTE", "SUBMISSIONS_PER_MINUTE"] {
        assert_eq!(
            拒绝原因(&[(名称, "0")]),
            "会话有效期须为 1..=2592000 秒，限流次数必须大于零",
            "{名称} 不接受 0"
        );
        let 期望 = format!("{名称} 必须是正整数");
        assert_eq!(拒绝原因(&[(名称, "很多")]), 期望, "{名称} 不接受非整数");
        assert_eq!(拒绝原因(&[(名称, "-1")]), 期望, "{名称} 不接受负数");
    }
}

#[test]
fn 不可解码的环境变量被视为非法文本() {
    let error = Config::from_lookup(move |名称| match 名称 {
        "DATABASE_URL" => Ok(数据库.to_owned()),
        _ => Err(env::VarError::NotUnicode("非 UTF-8 值".into())),
    })
    .err()
    .expect("不可解码的可选配置必须拒绝");
    assert_eq!(error.to_string(), "PUBLIC_ORIGIN 不是有效文本");

    let error = Config::from_lookup(|_| Err(env::VarError::NotUnicode("非 UTF-8 值".into())))
        .err()
        .expect("不可解码的必填配置必须拒绝");
    assert_eq!(error.to_string(), "请设置 DATABASE_URL");
}
