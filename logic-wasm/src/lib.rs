//! 问卷 DSL 的 wasm 边界：把 `questionnaire-logic` 的纯逻辑以 JSON 字符串信封
//! 暴露给前端，替代 TS 侧的镜像实现。
//!
//! # 边界不变量
//!
//! - 所有导出的参数与返回值均为 String：入参是 JSON 文本，出参是信封
//!   `{"ok":…}` 或 `{"error":{code,path,question_id,line?,column?}}`。
//! - 数字精度经原文无损穿透：本 crate 与 `questionnaire-logic` 同用
//!   `serde_json` 的 `arbitrary_precision` 特性（workspace 锁定同一版本），
//!   JS 侧配合 lossless-json 即可保留数字原文。
//! - 错误对象由 serde 序列化 `LogicError` / `LogicTextError` 得到，不手拼 JSON，
//!   字段与核心错误结构一一对应、无信息丢失。
//! - 信封本身也由 `serde_json::to_string` 产生；只有「入参不是合法 JSON / 不是
//!   预期 DTO」这一层错误是本 crate 自己的 `invalid_syntax` 诊断。

use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
use questionnaire_logic::domain::{
    FlowEvent, FlowSession as CoreFlowSession,
};
use questionnaire_logic::domain::{
    Document, DocumentLogic, Question, compile_expression as core_compile_expression,
    format_logic_text as core_format_logic_text, parse_logic_text as core_parse_logic_text,
};
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

/// 反序列化入参：解析失败与领域错误同样走 error 信封，但不携带题目/位置。
fn decode<'de, T: serde::Deserialize<'de>>(raw: &'de str, what: &str) -> Result<T, Value> {
    serde_json::from_str(raw)
        .map_err(|error| boundary_error(format!("{what} 不是合法 JSON：{error}")))
}

/// 领域错误统一序列化为信封错误对象；错误对象本身只含基础类型，序列化不会失败。
fn domain_error(error: impl serde::Serialize) -> Value {
    serde_json::to_value(&error).unwrap_or_else(|_| boundary_error("领域错误序列化失败"))
}

/// 解析文本 DSL；失败信封额外携带 `line` / `column`。
#[wasm_bindgen]
pub fn parse_logic_text(source: String, questions_json: String) -> String {
    let result = (|| {
        let questions: Vec<Question> = decode(&questions_json, "questions_json")?;
        let logic = core_parse_logic_text(&source, &questions).map_err(domain_error)?;
        serde_json::to_value(&logic).map_err(serde_json_to_boundary)
    })();
    envelope_generic(result)
}

/// 序列化成功值失败只可能是信封构造故障，统一转为边界错误。
fn serde_json_to_boundary(error: serde_json::Error) -> Value {
    boundary_error(format!("结果序列化失败：{error}"))
}
/// 把规则 JSON 格式化回文本 DSL。
#[wasm_bindgen]
pub fn format_logic_text(logic_json: String, questions_json: String) -> String {
    let result = (|| {
        let questions: Vec<Question> = decode(&questions_json, "questions_json")?;
        let logic: DocumentLogic = decode(&logic_json, "logic_json")?;
        let text = core_format_logic_text(&logic, &questions).map_err(domain_error)?;
        serde_json::to_value(&text).map_err(serde_json_to_boundary)
    })();
    envelope_generic(result)
}

/// 保存前的诊断预检：编译整份规则，只关心错误里的 `path` / `question_id`。
#[wasm_bindgen]
pub fn compile_document_logic(logic_json: String, questions_json: String) -> String {
    let result = (|| {
        let questions: Vec<Question> = decode(&questions_json, "questions_json")?;
        let logic: DocumentLogic = decode(&logic_json, "logic_json")?;
        logic
            .compile(&questions)
            .map(|_| json!({ "compiled": true }))
            .map_err(domain_error)
    })();
    envelope_generic(result)
}

/// 编译单个表达式包：`{"value_type":…,"dependencies":[…]}`，供表达式编辑器提示。
#[wasm_bindgen]
pub fn compile_expression(raw_json: String, questions_json: String) -> String {
    let result = (|| {
        let questions: Vec<Question> = decode(&questions_json, "questions_json")?;
        let raw: Value = serde_json::from_str(&raw_json)
            .map_err(|error| boundary_error(format!("raw_json 不是合法 JSON：{error}")))?;
        let compiled = core_compile_expression(&raw, &questions).map_err(domain_error)?;
        let dependencies: Vec<_> = compiled.dependencies().collect();
        serde_json::to_value(json!({
            "value_type": compiled.value_type(),
            "dependencies": dependencies,
        }))
        .map_err(serde_json_to_boundary)
    })();
    envelope_generic(result)
}
/// 重写整份问卷的身份（复制问卷/规则时使用）。
#[wasm_bindgen]
pub fn renew_document_ids(document_json: String) -> String {
    let result = (|| {
        let mut document: Document = decode(&document_json, "document_json")?;
        document.renew_ids();
        serde_json::to_value(&document).map_err(serde_json_to_boundary)
    })();
    envelope_generic(result)
}

/// 有状态的填写会话：文档与状态留在 wasm 线性内存，避免每次改答重编译。
///
/// 核心的 `FlowSession<'a>` 借用 `&'a Document`，而 wasm-bindgen 导出类只能持
/// owned 字段，两者不能直接组合。这里的取舍：文档放进 `Box<Document>`（地址
/// 稳定、随会话 Drop 一并释放，不使用 `Box::leak`——那会让每次创建会话都
/// 永久泄漏一份文档），再把该引用的生命周期安全地延伸为 `'static` 交给核心
/// 会话。安全性由三条不变量保证：
///
/// 1. `Box` 分配后地址不再变化：`FlowSession` 本身被 wasm-bindgen 放在堆上且
///    本 crate 从不移动 `document` 字段（无 `mem::replace`/移动构造）；
/// 2. 字段声明顺序 `session` 在 `document` 之前，Drop 时先拆掉会话再释放文档，
///    借用永远不悬空；
/// 3. 会话只在 `new` / `dispatch` / `snapshot` 内使用，期间 `self` 可变/不可变
///    借用始终有效。
///
/// 仅在 wasm32 目标导出：wasm-bindgen 的类导出 wrapper 在原生目标会 panic
/// （“function not implemented on non-wasm32 targets”），纯函数无此限制。
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub struct FlowSession {
    session: Option<CoreFlowSession<'static>>,
    document: Box<Document>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for FlowSession {
    fn drop(&mut self) {
        // 先于 document 释放（字段顺序已保证，这里显式表达意图）。
        self.session.take();
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl FlowSession {
    /// 绑定文档并创建空会话；失败时返回 JSON 错误字符串（非信封，见计划约定）。
    #[wasm_bindgen(constructor)]
    pub fn new(document_json: String) -> Result<FlowSession, JsValue> {
        let document: Document = serde_json::from_str(&document_json).map_err(|error| {
            JsValue::from_str(&serde_json::to_string(&json!({
                "error": boundary_error(format!("document_json 不是合法 JSON：{error}"))
            })).expect("信封序列化不会失败"))
        })?;
        let mut this = Self {
            session: None,
            document: Box::new(document),
        };
        // SAFETY：`this.document` 是 Box 指向的堆分配，地址稳定且 Drop 晚于
        // `session`（见结构体文档），把引用生命周期延伸为 'static 不会悬空。
        let document: &Document = unsafe { &*(&*this.document as *const Document) };
        let session = CoreFlowSession::new(document).map_err(|error| {
            JsValue::from_str(
                &serde_json::to_string(&json!({
                    "error": serde_json::to_value(&error).expect("错误序列化不会失败")
                }))
                .expect("信封序列化不会失败"),
            )
        })?;
        this.session = Some(session);
        Ok(this)
    }

    /// 应用一个流程事件；事件 JSON 反序列化失败或流程拒绝时返回 error 信封。
    pub fn dispatch(&mut self, event_json: String) -> String {
        let result = (|| {
            let event: FlowEvent = serde_json::from_str(&event_json)
                .map_err(|error| boundary_error(format!("event_json 不是合法事件：{error}")))?;
            let session = self
                .session
                .as_mut()
                .expect("session 在 new 成功后始终存在");
            session.dispatch(event).map_err(domain_error)
        })();
        envelope_generic(result.map(|()| Value::Null))
    }

    /// 只读快照，序列化为 `{"ok":snapshot}`。
    pub fn snapshot(&self) -> String {
        let snapshot = self
            .session
            .as_ref()
            .expect("session 在 new 成功后始终存在")
            .snapshot();
        serde_json::to_string(&json!({
            "ok": serde_json::to_value(&snapshot).expect("快照序列化不会失败")
        }))
        .expect("信封序列化不会失败")
    }
}

/// 统一信封：错误在进入本函数前已序列化为 `serde_json::Value`（领域错误经
/// serde 序列化，边界错误由 `boundary_error` 构造），成功值同样为 Value；
/// 收口到一处保证 `{"ok":…} | {"error":…}` 的形状唯一。
fn envelope_generic(result: Result<Value, Value>) -> String {
    match result {
        Ok(value) => serde_json::to_string(&json!({ "ok": value })).expect("信封序列化不会失败"),
        Err(error) => {
            serde_json::to_string(&json!({ "error": error })).expect("信封序列化不会失败")
        }
    }
}
