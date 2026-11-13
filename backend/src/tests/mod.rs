//! 后端单元测试，按生产模块的职责边界拆分。
//!
//! | 模块 | 覆盖的边界 |
//! | --- | --- |
//! | `config` | 环境变量解析、默认值与启动期安全检查 |
//! | `error_and_http` | 错误码、响应头与提取器拒绝 |
//! | `security_and_router` | CSRF、固定窗口限流、路由与统一响应头 |
//! | `auth_and_admin` | 账号凭据、Cookie 解析与管理员治理校验 |
//! | `surveys_and_submissions` | 状态机、回收资格与公开提交契约 |
//! | `reports` | 时间范围、持久化三态、统计聚合与分批 CSV |
//!
//! 所有用例纯内存运行，不需要 PostgreSQL；需要协议层行为时用 `support::send_raw_http`
//! 走真实的 TCP 与 `axum::serve`。

mod auth_and_admin;
mod config;
mod error_and_http;
mod reports;
mod security_and_router;
mod support;
mod surveys_and_submissions;
