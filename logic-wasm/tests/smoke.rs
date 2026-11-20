//! 原生冒烟测试：不走 wasm 目标，直接调用与 `#[wasm_bindgen]` 导出完全相同的纯函数体。
//!
//! 本文件只守 JSON 字符串信封边界：成功值与错误值互斥、每个字符串入参的解码失败
//! 都落到 `invalid_syntax` 边界错误，以及领域错误字段（含行列）原样穿透。
//! 夹具的语义细节由 `logic-core` 的集成测试负责，这里不再重复断言。

use serde_json::{Value, json};

#[path = "../../logic-core/tests/support/mod.rs"]
mod support;

fn diamond() -> Value {
    support::flows()["documents"]["diamond"].clone()
}

fn expected_text() -> String {
    support::texts()["format_cases"][0]["expected"]
        .as_str()
        .unwrap()
        .to_string()
}

fn parse_envelope(output: &str) -> Value {
    serde_json::from_str(output).expect("信封必须是合法 JSON")
}

/// 成功信封只含 `ok`，且不得携带 `error` 键。
fn ok_envelope(output: &str) -> Value {
    let envelope = parse_envelope(output);
    assert!(
        envelope.get("error").is_none(),
        "成功信封不得包含 error：{envelope}"
    );
    let fields = envelope.as_object().expect("信封必须是对象");
    assert_eq!(fields.len(), 1, "成功信封只允许一个键：{envelope}");
    envelope["ok"].clone()
}

/// 边界解码失败信封：固定分类、根路径、无题目身份，且消息点名入参。
fn boundary_envelope(output: &str, parameter: &str) -> Value {
    let envelope = parse_envelope(output);
    assert!(
        envelope.get("ok").is_none(),
        "失败信封不得携带 ok：{envelope}"
    );
    let error = &envelope["error"];
    assert_eq!(error["code"], "invalid_syntax", "{envelope}");
    assert_eq!(error["path"], "$", "{envelope}");
    assert!(error["question_id"].is_null(), "{envelope}");
    let message = error["message"].as_str().expect("边界消息必须是字符串");
    assert!(
        message.contains(parameter),
        "边界消息必须点名参数 {parameter}：{message}"
    );
    envelope
}

/// 领域错误信封：只校验信封形状，字段由各测试按领域契约断言。
fn domain_envelope(output: &str) -> Value {
    let envelope = parse_envelope(output);
    assert!(
        envelope.get("ok").is_none(),
        "领域失败信封不得携带 ok：{envelope}"
    );
    envelope["error"].clone()
}

#[test]
fn 四个导出函数成功时只返回_ok_信封() {
    let expected = expected_text();
    let diamond = diamond();
    let questions = diamond["questions"].to_string();
    let logic = diamond["logic"].to_string();

    // 1. 文本解析：成功值是规则包对象。
    let parsed = ok_envelope(&logic_wasm::parse_logic_text(&expected, &questions));
    assert!(parsed.is_object(), "{parsed}");

    // 2. 文本格式化：成功值是 DSL 字符串。
    let formatted = ok_envelope(&logic_wasm::format_logic_text(&logic, &questions));
    assert!(
        formatted.is_string(),
        "格式化成功值必须是字符串：{formatted}"
    );

    // 3. 文档规则预检：成功值是编译结果对象，携带变量拓扑顺序。
    let compiled = ok_envelope(&logic_wasm::compile_document_logic(&logic, &questions));
    assert!(compiled["variable_order"].is_array(), "{compiled}");

    // 4. 文本巡检：成功值携带绑定列表与可选诊断字段。
    let inspected = ok_envelope(&logic_wasm::inspect_logic_text(&expected, &questions));
    assert!(inspected["bindings"].is_array(), "{inspected}");
    assert!(inspected.get("error").is_some(), "{inspected}");
    assert!(inspected.get("error_offset").is_some(), "{inspected}");
}

#[test]
fn 非法_json_与错误_dto_结构按参数名归类为边界语法错误() {
    let expected = expected_text();
    let diamond = diamond();
    let questions = diamond["questions"].to_string();
    let logic = diamond["logic"].to_string();
    let syntax = "{ 非法";
    let wrong_questions = "{}";
    let wrong_logic = "[]";

    // 每个 (function, parameter) 组合保留一行；语法错误与结构错误交替覆盖。
    let cases = [
        (
            "questions_json",
            logic_wasm::parse_logic_text(&expected, syntax),
        ),
        (
            "questions_json",
            logic_wasm::inspect_logic_text(&expected, syntax),
        ),
        (
            "questions_json",
            logic_wasm::inspect_logic_text(&expected, wrong_questions),
        ),
        (
            "logic_json",
            logic_wasm::format_logic_text(wrong_logic, &questions),
        ),
        (
            "questions_json",
            logic_wasm::format_logic_text(&logic, syntax),
        ),
        (
            "logic_json",
            logic_wasm::compile_document_logic(wrong_logic, &questions),
        ),
        (
            "questions_json",
            logic_wasm::compile_document_logic(&logic, syntax),
        ),
    ];
    for (parameter, output) in cases {
        boundary_envelope(&output, parameter);
    }
}

#[test]
fn 领域错误字段原样穿透信封() {
    let diamond = diamond();
    let questions_json = diamond["questions"].to_string();
    let question_id = diamond["questions"][0]["id"].as_str().unwrap();

    // 语法错误来自文本解析，额外携带从 1 开始的行列号。
    let error = domain_envelope(&logic_wasm::parse_logic_text("@@", &questions_json));
    assert_eq!(error["code"], "invalid_syntax", "{error}");
    assert_eq!(error["path"], "$", "{error}");
    assert!(error["question_id"].is_null(), "{error}");
    assert!(
        error["line"].as_u64().is_some_and(|line| line >= 1),
        "{error}"
    );
    assert!(
        error["column"].as_u64().is_some_and(|column| column >= 1),
        "{error}"
    );

    // 文档规则错误保留规则路径与相关题目，且不携带行列。
    let logic = json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "variables": [],
        "rules": [{
            "question_id": question_id,
            "visible_if": { "op": "answer", "question_id": "00000000-0000-4000-8000-000000000999" },
        }],
        "flow": null,
    })
    .to_string();
    let error = domain_envelope(&logic_wasm::compile_document_logic(&logic, &questions_json));
    assert_eq!(error["code"], "invalid_reference", "{error}");
    assert_eq!(error["path"], "$.logic.rules[0].visible_if", "{error}");
    assert_eq!(
        error["question_id"], "00000000-0000-4000-8000-000000000999",
        "{error}"
    );
    assert!(error.get("line").is_none(), "领域错误不带行列：{error}");
    assert!(error.get("column").is_none(), "领域错误不带行列：{error}");
}

#[test]
fn 巡检信封在语法错误后仍返回已扫描绑定() {
    let diamond = diamond();
    let questions = diamond["questions"].to_string();
    let question_id = diamond["questions"][0]["id"].as_str().unwrap();
    let source =
        format!("dsl 1 semantics 1;\nbind question q = \"{question_id}\";\nlet broken = ;");
    let inspection = ok_envelope(&logic_wasm::inspect_logic_text(&source, &questions));
    let bindings = inspection["bindings"].as_array().expect("必须返回数组");
    assert_eq!(bindings.len(), 1, "{inspection}");
    assert_eq!(bindings[0]["category"], "question");
    assert_eq!(bindings[0]["alias"], "q");
    assert_eq!(bindings[0]["id"], question_id);
    assert_eq!(
        inspection["error"]["code"], "invalid_syntax",
        "{inspection}"
    );
    assert!(
        inspection["error_offset"].as_u64().is_some(),
        "错误必须带字节偏移：{inspection}"
    );
    assert!(inspection["line"].as_u64().is_some_and(|line| line >= 1));
    assert!(
        inspection["column"]
            .as_u64()
            .is_some_and(|column| column >= 1)
    );
}
