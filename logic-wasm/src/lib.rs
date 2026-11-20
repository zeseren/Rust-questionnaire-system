//! 问卷 DSL 的 wasm 边界：把 `questionnaire-logic` 的纯逻辑以 JSON 字符串信封
//! 暴露给前端，替代 TS 侧的镜像实现。
//!
//! # 边界不变量
//!
//! - 纯函数导出以借用字符串（`&str`）接收 JSON 文本，返回值是 `String` 信封
//!   `{"ok":…}` 或 `{"error":{code,path,question_id,line?,column?}}`。
//! - 数字按标准有限 `f64` 语义穿透 JSON 信封，与 JS 原生 `number` 直接对应。
//! - 错误对象由 serde 序列化 `LogicError` / `LogicTextError` 得到，不手拼 JSON，
//!   字段与核心错误结构一一对应、无信息丢失。
//! - 信封本身也由 `serde_json::to_string` 产生；只有「入参不是合法 JSON / 不是
//!   预期 DTO」这一层错误是本 crate 自己的 `invalid_syntax` 诊断。

use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
use questionnaire_logic::domain::{Document, FlowEvent, FlowSession as CoreFlowSession};
use questionnaire_logic::domain::{
    DocumentLogic, Question, format_logic_text as core_format_logic_text,
    inspect_logic_text as core_inspect_logic_text, parse_logic_text as core_parse_logic_text,
};
use questionnaire_logic::identity::Uuid;

/// 边界自身的诊断（非领域错误）：入参反序列化失败等。
///
/// message 用 String 而非 &'static str：serde 的文本不属于领域错误表。
fn boundary_error(message: impl Into<String>) -> Value {
    json!({
        "code": "invalid_syntax",
        "path": "$",
        "question_id": Value::Null,
        "message": message.into(),
    })
}

/// 反序列化入参：语法/IO 失败与数据类型契约失败使用不同文案，均走 error 信封，
/// 但不携带题目/位置。
fn decode<'de, T: serde::Deserialize<'de>>(raw: &'de str, what: &str) -> Result<T, Value> {
    serde_json::from_str(raw).map_err(|error| {
        let message = match error.classify() {
            serde_json::error::Category::Syntax
            | serde_json::error::Category::Eof
            | serde_json::error::Category::Io => format!("{what} 不是合法 JSON：{error}"),
            serde_json::error::Category::Data => format!("{what} 不符合数据契约：{error}"),
        };
        boundary_error(message)
    })
}

/// 领域错误统一序列化为信封错误对象；错误对象本身只含基础类型，序列化不会失败。
fn domain_error(error: impl serde::Serialize) -> Value {
    serde_json::to_value(&error).expect("领域错误只含基础类型，序列化不应失败")
}

/// 解析文本 DSL；失败信封额外携带 `line` / `column`。
#[wasm_bindgen]
#[must_use]
pub fn parse_logic_text(source: &str, questions_json: &str) -> String {
    let result = (|| {
        let questions: Vec<Question> = decode(questions_json, "questions_json")?;
        core_parse_logic_text(source, &questions).map_err(domain_error)
    })();
    envelope_generic(result)
}

/// 巡检文本 DSL：成功信封返回 `{bindings,error,error_offset,line,column}`。
///
/// 解析失败时错误保留在成功值内部，而不是整体变成 error 信封，这样前端右侧速查栏
/// 可以同时展示已扫描到的绑定；只有 `questions_json` 本身非法才返回边界错误。
#[wasm_bindgen]
#[must_use]
pub fn inspect_logic_text(source: &str, questions_json: &str) -> String {
    let result = (|| {
        let questions: Vec<Question> = decode(questions_json, "questions_json")?;
        Ok(core_inspect_logic_text(source, &questions))
    })();
    envelope_generic(result)
}

/// 把规则 JSON 格式化回文本 DSL。
#[wasm_bindgen]
#[must_use]
pub fn format_logic_text(logic_json: &str, questions_json: &str) -> String {
    let result = (|| {
        let questions: Vec<Question> = decode(questions_json, "questions_json")?;
        let logic: DocumentLogic = decode(logic_json, "logic_json")?;
        core_format_logic_text(&logic, &questions).map_err(domain_error)
    })();
    envelope_generic(result)
}

/// `compile_document_logic` 的成功响应：计算变量的拓扑顺序。
#[derive(serde::Serialize)]
struct CompiledDocumentLogicResponse {
    variable_order: Vec<Uuid>,
}

/// 保存前的诊断预检：编译整份规则，只关心错误里的 `path` / `question_id`。
///
/// 成功信封同时返回 `variable_order`：计算变量的拓扑顺序，供前端在编译期展示
/// 与断言使用。
#[wasm_bindgen]
#[must_use]
pub fn compile_document_logic(logic_json: &str, questions_json: &str) -> String {
    let result = (|| {
        let questions: Vec<Question> = decode(questions_json, "questions_json")?;
        let logic: DocumentLogic = decode(logic_json, "logic_json")?;
        let compiled = logic.compile(&questions).map_err(domain_error)?;
        Ok(CompiledDocumentLogicResponse {
            variable_order: compiled.variable_order().to_vec(),
        })
    })();
    envelope_generic(result)
}

/// 有状态的填写会话：文档与状态留在 wasm 线性内存，避免每次改答重编译。
///
/// 核心的 `FlowSession` 在构造时接管 `Document` 的所有权，导出类只需持有
/// 会话本身，借用生命周期与指针稳定性都由核心类型自行保证。
///
/// 仅在 wasm32 目标导出：wasm-bindgen 的类导出 wrapper 在原生目标会 panic
/// （“function not implemented on non-wasm32 targets”），纯函数无此限制。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub struct FlowSession {
    session: CoreFlowSession,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl FlowSession {
    /// 解析文档、接管所有权并创建空会话；失败时抛出携带 `{"error":…}` 信封 JSON
    /// 的字符串（与其它导出同一信封形状）。
    #[wasm_bindgen(constructor)]
    pub fn new(document_json: String) -> Result<FlowSession, JsValue> {
        let document: Document = decode(&document_json, "document_json")
            .map_err(|error| JsValue::from_str(&envelope_generic::<Value>(Err(error))))?;
        let session = CoreFlowSession::from_document(document).map_err(|error| {
            JsValue::from_str(&envelope_generic::<Value>(Err(domain_error(error))))
        })?;
        Ok(Self { session })
    }

    /// 应用一个流程事件；事件 JSON 反序列化失败或流程拒绝时返回 error 信封。
    pub fn dispatch(&mut self, event_json: String) -> String {
        let result = (|| {
            let event: FlowEvent = serde_json::from_str(&event_json)
                .map_err(|error| boundary_error(format!("event_json 不是合法事件：{error}")))?;
            self.session.dispatch(event).map_err(domain_error)
        })();
        envelope_generic(result.map(|()| Value::Null))
    }

    /// 只读快照，序列化为 `{"ok":snapshot}`；借用会话集合，避免每次改答深拷贝。
    pub fn snapshot(&self) -> String {
        envelope_generic(Ok(self.session.snapshot()))
    }
}

/// 信封只含一个键：成功值与错误值互斥，错误在进入本函数前已序列化为
/// `serde_json::Value`（领域错误经 serde 序列化，边界错误由 `boundary_error` 构造）。
#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Envelope<T> {
    Ok(T),
    Error(Value),
}

/// 统一信封：把成功值或错误值一次性序列化成 `{"ok":…}` / `{"error":…}`，
/// 形状只在 `Envelope` 一处定义，成功值不需要先转成 `Value` 再序列化。
fn envelope_generic<T: serde::Serialize>(result: Result<T, Value>) -> String {
    let envelope = match result {
        Ok(value) => Envelope::Ok(value),
        Err(error) => Envelope::Error(error),
    };
    serde_json::to_string(&envelope).expect("信封只由可序列化的领域值组成，序列化不应失败")
}
