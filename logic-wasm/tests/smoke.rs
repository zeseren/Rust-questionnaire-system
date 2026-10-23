//! 原生冒烟测试：不走 wasm 目标，直接调用与 `#[wasm_bindgen]` 导出完全相同的纯函数体。
//!
//! 本文件只守 JSON 字符串信封边界：成功值与错误值互斥、每个字符串入参的解码失败
//! 都落到 `invalid_syntax` 边界错误、严格答案键，以及领域错误字段（含行列）原样穿透。
//! 夹具的语义细节由 `logic-core` 的集成测试负责，这里不再重复断言。

use serde_json::{Value, json};

fn texts() -> Value {
    serde_json::from_str(include_str!("../../fixtures/survey-logic/texts.json"))
        .expect("共享文本样例必须是合法 JSON")
}

fn expressions() -> Value {
    serde_json::from_str(include_str!("../../fixtures/survey-logic/expressions.json"))
        .expect("共享表达式样例必须是合法 JSON")
}

fn documents() -> Value {
    serde_json::from_str(include_str!("../../fixtures/survey-logic/documents.json"))
        .expect("共享文档样例必须是合法 JSON")
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
fn 七个导出函数成功时只返回_ok_信封() {
    let texts = texts();
    let item = &texts["format_cases"][0];
    let questions = item["questions"].to_string();
    let logic = item["logic"].to_string();
    let expected = item["expected"].as_str().unwrap().to_string();

    // 1. 文本解析：成功值是规则包对象。
    let parsed = ok_envelope(&logic_wasm::parse_logic_text(
        expected.clone(),
        questions.clone(),
    ));
    assert!(parsed.is_object(), "{parsed}");

    // 2. 文本格式化：成功值是 DSL 字符串。
    let formatted = ok_envelope(&logic_wasm::format_logic_text(
        logic.clone(),
        questions.clone(),
    ));
    assert!(
        formatted.is_string(),
        "格式化成功值必须是字符串：{formatted}"
    );

    // 3. 文档规则预检：成功值是编译结果对象。
    let expressions = expressions();
    let expression_questions = expressions["questions"].to_string();
    let documents = documents();
    let case = &documents["cases"][0];
    let case_logic = case["logic"].to_string();
    let compiled = ok_envelope(&logic_wasm::compile_document_logic(
        case_logic.clone(),
        expression_questions.clone(),
    ));
    assert!(compiled.is_object(), "{compiled}");

    // 4. 表达式编译：成功值是编译结果对象。
    let raw = json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "expression": { "op": "answered", "question_id": expression_questions_id(&expressions) },
    })
    .to_string();
    let compiled = ok_envelope(&logic_wasm::compile_expression(
        raw.clone(),
        expression_questions.clone(),
    ));
    assert!(compiled.is_object(), "{compiled}");

    // 5. 身份重写：成功值是文档对象。
    let document = json!({
        "title": "冒烟",
        "description": "",
        "purpose": "",
        "thank_you": "",
        "logic": item["logic"],
        "questions": item["questions"],
    })
    .to_string();
    let renewed = ok_envelope(&logic_wasm::renew_document_ids(document));
    assert!(renewed.is_object(), "{renewed}");

    // 6. 单表达式求值：成功值是规则值对象。
    let evaluated = ok_envelope(&logic_wasm::evaluate_expression(
        raw,
        expression_questions.clone(),
        "{}".to_string(),
    ));
    assert!(evaluated.is_object(), "{evaluated}");

    // 7. 文档规则求值：成功值是求值结果对象。
    let evaluated = ok_envelope(&logic_wasm::evaluate_document_logic(
        case_logic,
        expression_questions,
        case["answers_json"].as_str().unwrap().to_string(),
    ));
    assert!(evaluated.is_object(), "{evaluated}");
}

fn expression_questions_id(expressions: &Value) -> &str {
    expressions["questions"][0]["id"].as_str().unwrap()
}

#[test]
fn 非法_json_与错误_dto_结构按参数名归类为边界语法错误() {
    let texts = texts();
    let item = &texts["format_cases"][0];
    let questions = item["questions"].to_string();
    let logic = item["logic"].to_string();
    let expected = item["expected"].as_str().unwrap().to_string();
    let expressions = expressions();
    let expression_questions = expressions["questions"].to_string();
    let documents = documents();
    let case = &documents["cases"][0];
    let case_logic = case["logic"].to_string();
    let case_answers = case["answers_json"].as_str().unwrap().to_string();
    let raw = json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "expression": { "op": "boolean", "value": true },
    })
    .to_string();
    let answers = "{}".to_string();
    let syntax = "{ 非法".to_string();
    let wrong_questions = "{}".to_string();
    let wrong_logic = "[]".to_string();
    let wrong_answers = "[]".to_string();
    let wrong_document = "[]".to_string();

    // 每个 (function, parameter) 组合保留一行；语法错误与结构错误交替覆盖。
    let cases = [
        (
            "questions_json",
            logic_wasm::parse_logic_text(expected, syntax.clone()),
        ),
        (
            "logic_json",
            logic_wasm::format_logic_text(wrong_logic.clone(), questions.clone()),
        ),
        (
            "questions_json",
            logic_wasm::format_logic_text(logic.clone(), syntax.clone()),
        ),
        (
            "logic_json",
            logic_wasm::compile_document_logic(wrong_logic.clone(), questions.clone()),
        ),
        (
            "questions_json",
            logic_wasm::compile_document_logic(logic.clone(), syntax.clone()),
        ),
        (
            "raw_json",
            logic_wasm::compile_expression(syntax.clone(), questions.clone()),
        ),
        (
            "questions_json",
            logic_wasm::compile_expression(raw.clone(), wrong_questions.clone()),
        ),
        (
            "raw_json",
            logic_wasm::evaluate_expression(syntax.clone(), questions.clone(), answers.clone()),
        ),
        (
            "questions_json",
            logic_wasm::evaluate_expression(raw.clone(), wrong_questions.clone(), answers.clone()),
        ),
        (
            "answers_json",
            logic_wasm::evaluate_expression(raw, questions.clone(), syntax.clone()),
        ),
        (
            "logic_json",
            logic_wasm::evaluate_document_logic(
                wrong_logic,
                expression_questions.clone(),
                case_answers.clone(),
            ),
        ),
        (
            "questions_json",
            logic_wasm::evaluate_document_logic(case_logic.clone(), syntax.clone(), case_answers),
        ),
        (
            "answers_json",
            logic_wasm::evaluate_document_logic(case_logic, expression_questions, wrong_answers),
        ),
        (
            "document_json",
            logic_wasm::renew_document_ids(wrong_document),
        ),
    ];
    for (parameter, output) in cases {
        boundary_envelope(&output, parameter);
    }
}

#[test]
fn 严格答案键拒绝非规范_uuid() {
    let expressions = expressions();
    let questions = expressions["questions"].to_string();
    let raw = json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "expression": { "op": "answered", "question_id": expression_questions_id(&expressions) },
    })
    .to_string();
    let logic = documents()["cases"][0]["logic"].to_string();

    // 全零 UUID 在解码阶段拒绝；规范键由下面的对照覆盖。
    let answers = r#"{"00000000-0000-0000-0000-000000000000":"x"}"#.to_string();
    for output in [
        logic_wasm::evaluate_expression(raw.clone(), questions.clone(), answers.clone()),
        logic_wasm::evaluate_document_logic(logic.clone(), questions.clone(), answers.clone()),
    ] {
        boundary_envelope(&output, "answers_json");
    }

    // 对照：规范键与合法选项值进入领域层并正常求值。
    let valid_option = expressions["questions"][0]["options"][0]["id"]
        .as_str()
        .unwrap();
    let answers = format!(
        r#"{{"{}":"{valid_option}"}}"#,
        expression_questions_id(&expressions)
    );
    let envelope = parse_envelope(&logic_wasm::evaluate_expression(raw, questions, answers));
    assert!(
        envelope.get("error").is_none(),
        "规范 UUID 键不应触发任何错误：{envelope}"
    );
    assert_eq!(envelope["ok"], json!({ "type": "boolean", "value": true }));
}

#[test]
fn 领域错误字段原样穿透信封() {
    let expressions = expressions();
    let questions_json = expressions["questions"].to_string();
    let question_id = expression_questions_id(&expressions);

    // 语法错误来自文本解析，额外携带从 1 开始的行列号。
    let error = domain_envelope(&logic_wasm::parse_logic_text(
        "@@".to_string(),
        questions_json.clone(),
    ));
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
    let error = domain_envelope(&logic_wasm::compile_document_logic(logic, questions_json));
    assert_eq!(error["code"], "invalid_reference", "{error}");
    assert_eq!(error["path"], "$.logic.rules[0].visible_if", "{error}");
    assert_eq!(
        error["question_id"], "00000000-0000-4000-8000-000000000999",
        "{error}"
    );
    assert!(error.get("line").is_none(), "领域错误不带行列：{error}");
    assert!(error.get("column").is_none(), "领域错误不带行列：{error}");
}
