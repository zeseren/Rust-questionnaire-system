//! 问卷领域 DSL 核心 crate（`questionnaire-logic`）。
//!
//! 维护的不变量：纯领域逻辑，不含 HTTP / 数据库 / 文件系统等外部依赖，
//! 可编译到 wasm32-unknown-unknown 供前端复用同一套校验规则。
//! 对外入口只有 `domain`（聚合再导出）与 `identity`；其余模块经 `domain::*` 暴露。
mod flow;
mod logic;
mod logic_document;
mod logic_text;

pub mod domain;
pub mod identity;
