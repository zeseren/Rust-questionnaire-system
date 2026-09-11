//! 环境变量是启动配置；配置错误直接停止启动，不悄悄切换数据库或安全策略。

use std::{env, net::SocketAddr};

use anyhow::{Context, bail};

/// HTTP 服务、数据库和浏览器安全设置。
pub struct Config {
    /// PostgreSQL 连接串，只在服务端使用。
    pub database_url: String,
    /// 服务监听地址。
    pub listen: SocketAddr,
    /// 浏览器访问站点的完整 origin，不含路径。
    pub public_origin: String,
    /// HTTPS 部署必须为 true；仅本机 HTTP 开发允许 false。
    pub cookie_secure: bool,
    /// 登录会话有效期（秒）。
    pub session_ttl_seconds: i64,
    /// 整个单实例每分钟允许的注册/登录请求数。
    pub auth_requests_per_minute: u32,
    /// 整个单实例每分钟允许的公开提交请求数。
    pub submissions_per_minute: u32,
}

impl Config {
    /// 读取配置。可选项只在未设置时采用文档中的开发默认值。
    ///
    /// # Errors
    /// 必填值缺失、值格式错误或 HTTP/HTTPS 与 Cookie 安全设置矛盾时失败。
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = env::var("DATABASE_URL").context("请设置 DATABASE_URL")?;
        let database = url::Url::parse(&database_url).context("DATABASE_URL 格式错误")?;
        if !matches!(database.scheme(), "postgres" | "postgresql") {
            bail!("DATABASE_URL 必须使用 PostgreSQL");
        }
        let public_origin = value("PUBLIC_ORIGIN", "http://localhost:3000")?;
        let origin = url::Url::parse(&public_origin).context("PUBLIC_ORIGIN 格式错误")?;
        if !matches!(origin.scheme(), "http" | "https")
            || origin.host_str().is_none()
            || origin.origin().ascii_serialization() != public_origin
        {
            bail!("PUBLIC_ORIGIN 必须是无路径的 http(s) origin，例如 http://localhost:3000");
        }
        let cookie_secure: bool = value("COOKIE_SECURE", "false")?
            .parse()
            .context("COOKIE_SECURE 必须为 true 或 false")?;
        if origin.scheme() == "https" && !cookie_secure {
            bail!("HTTPS 部署要求 COOKIE_SECURE=true");
        }
        if origin.scheme() == "http" && cookie_secure {
            bail!("HTTP 无法使用 Secure Cookie；请使用 HTTPS 或本机开发设置");
        }
        if origin.scheme() == "http"
            && !matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        {
            bail!("非本机部署必须使用 HTTPS");
        }
        let config = Self {
            database_url,
            listen: value("LISTEN_ADDR", "127.0.0.1:3000")?
                .parse()
                .context("LISTEN_ADDR 必须是 IP:端口")?,
            public_origin,
            cookie_secure,
            session_ttl_seconds: value("SESSION_TTL_SECONDS", "86400")?.parse()?,
            auth_requests_per_minute: value("AUTH_REQUESTS_PER_MINUTE", "60")?.parse()?,
            submissions_per_minute: value("SUBMISSIONS_PER_MINUTE", "600")?.parse()?,
        };
        if !(1..=2_592_000).contains(&config.session_ttl_seconds)
            || config.auth_requests_per_minute == 0
            || config.submissions_per_minute == 0
        {
            bail!("会话有效期须为 1..=2592000 秒，限流次数必须大于零");
        }
        Ok(config)
    }
}

fn value(name: &str, default: &str) -> anyhow::Result<String> {
    match env::var(name) {
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Ok(default.to_owned()),
        Err(error) => Err(error).with_context(|| format!("{name} 不是有效文本")),
    }
}
