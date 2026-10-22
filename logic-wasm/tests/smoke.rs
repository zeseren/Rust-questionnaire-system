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

    // 1. 文本解析：成功值就是规则包对象。
    let parsed = ok_envelope(&logic_wasm::parse_logic_text(
        expected.clone(),
        questions.clone(),
    ));
    assert!(parsed["variables"].is_array(), "{parsed}");
    assert_eq!(parsed["dsl_version"], 1);

    // 2. 文本格式化：成功值是 DSL 字符串。
    let formatted = ok_envelope(&logic_wasm::format_logic_text(
        logic.clone(),
        questions.clone(),
    ));
    assert!(
        formatted.as_str().is_some_and(|text| !text.is_empty()),
        "格式化成功值必须是非空字符串：{formatted}"
    );

    // 3. 文档规则预检：variable_order 与输入变量一一对应。
    let expressions = expressions();
    let expression_questions = expressions["questions"].to_string();
    let documents = documents();
    let case = &documents["cases"][0];
    let case_logic = case["logic"].to_string();
    let compiled = ok_envelope(&logic_wasm::compile_document_logic(
        case_logic.clone(),
        expression_questions.clone(),
    ));
    let order = compiled["variable_order"]
        .as_array()
        .expect("编译成功必须返回 variable_order 数组");
    let variables = case["logic"]["variables"].as_array().unwrap();
    assert_eq!(order.len(), variables.len(), "{compiled}");
    assert!(order.iter().all(Value::is_string), "{compiled}");
    let mut ordered = order
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    ordered.sort();
    let mut expected_variables = variables
        .iter()
        .map(|variable| variable["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    expected_variables.sort();
    assert_eq!(ordered, expected_variables, "{compiled}");

    // 4. 表达式编译：返回静态类型与依赖列表。
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
    assert_eq!(compiled["value_type"], "boolean", "{compiled}");
    assert_eq!(
        compiled["dependencies"],
        json!([expression_questions_id(&expressions)]),
        "{compiled}"
    );

    // 5. 身份重写：成功值是结构完整的新文档，规则引用跟随题目身份。
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
    assert_eq!(renewed["title"], "冒烟");
    let renewed_questions = renewed["questions"].as_array().unwrap();
    assert_eq!(
        renewed_questions.len(),
        item["questions"].as_array().unwrap().len()
    );
    for (old, new) in item["questions"]
        .as_array()
        .unwrap()
        .iter()
        .zip(renewed_questions)
    {
        assert_ne!(new["id"], old["id"], "题目身份必须重写：{renewed}");
        assert!(new["id"].as_str().is_some_and(|id| id.len() == 36));
    }
    assert_eq!(
        renewed["logic"]["rules"][0]["question_id"], renewed_questions[1]["id"],
        "规则引用必须跟随题目身份重写"
    );
    let output = renewed.to_string();
    for old in item["questions"].as_array().unwrap() {
        assert!(
            !output.contains(old["id"].as_str().unwrap()),
            "旧题目身份不应残留：{output}"
        );
    }

    // 6. 单表达式求值：成功值是规则值对象。
    let evaluated = ok_envelope(&logic_wasm::evaluate_expression(
        raw,
        expression_questions.clone(),
        "{}".to_string(),
    ));
    assert!(
        evaluated.get("type").is_some_and(Value::is_string),
        "求值成功值必须带 type：{evaluated}"
    );

    // 7. 文档规则求值：成功值包含变量与规则两组结果。
    let evaluated = ok_envelope(&logic_wasm::evaluate_document_logic(
        case_logic,
        expression_questions,
        case["answers_json"].as_str().unwrap().to_string(),
    ));
    assert!(evaluated["variables"].is_object(), "{evaluated}");
    assert!(evaluated["rules"].is_object(), "{evaluated}");
}

fn expression_questions_id(expressions: &Value) -> &str {
    expressions["questions"][0]["id"].as_str().unwrap()
}

#[test]
fn 每个字符串入参的非法_json_都返回边界错误() {
    let texts = texts();
    let item = &texts["format_cases"][0];
    let questions = item["questions"].to_string();
    let logic = item["logic"].to_string();
    let expected = item["expected"].as_str().unwrap().to_string();
    let expressions = expressions();
    let documents = documents();
    let case = &documents["cases"][0];
    let raw = json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "expression": { "op": "boolean", "value": true },
    })
    .to_string();
    let answers = "{}".to_string();
    let bad = "{ 非法".to_string();

    let cases = [
        (
            "questions_json",
            logic_wasm::parse_logic_text(expected.clone(), bad.clone()),
        ),
        (
            "questions_json",
            logic_wasm::format_logic_text(logic.clone(), bad.clone()),
        ),
        (
            "questions_json",
            logic_wasm::compile_document_logic(logic.clone(), bad.clone()),
        ),
        (
            "questions_json",
            logic_wasm::compile_expression(raw.clone(), bad.clone()),
        ),
        (
            "questions_json",
            logic_wasm::evaluate_expression(raw.clone(), bad.clone(), answers.clone()),
        ),
        (
            "questions_json",
            logic_wasm::evaluate_document_logic(logic.clone(), bad.clone(), answers.clone()),
        ),
        (
            "logic_json",
            logic_wasm::format_logic_text(bad.clone(), questions.clone()),
        ),
        (
            "logic_json",
            logic_wasm::compile_document_logic(bad.clone(), questions.clone()),
        ),
        (
            "logic_json",
            logic_wasm::evaluate_document_logic(bad.clone(), questions.clone(), answers.clone()),
        ),
        (
            "raw_json",
            logic_wasm::compile_expression(bad.clone(), questions.clone()),
        ),
        (
            "raw_json",
            logic_wasm::evaluate_expression(bad.clone(), questions.clone(), answers.clone()),
        ),
        (
            "answers_json",
            logic_wasm::evaluate_expression(raw.clone(), questions.clone(), bad.clone()),
        ),
        (
            "answers_json",
            logic_wasm::evaluate_document_logic(
                case["logic"].to_string(),
                expressions["questions"].to_string(),
                bad.clone(),
            ),
        ),
        ("document_json", logic_wasm::renew_document_ids(bad)),
    ];
    for (parameter, output) in cases {
        boundary_envelope(&output, parameter);
    }
}

#[test]
fn 错误_dto_类型同样落到对应参数的边界错误() {
    let texts = texts();
    let item = &texts["format_cases"][0];
    let questions = item["questions"].to_string();
    let logic = item["logic"].to_string();
    let expected = item["expected"].as_str().unwrap().to_string();
    let expressions = expressions();
    let documents = documents();
    let case = &documents["cases"][0];
    let raw = json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "expression": { "op": "boolean", "value": true },
    })
    .to_string();
    let wrong_questions = "{}".to_string();
    let wrong_logic = "[]".to_string();
    let wrong_answers = "[]".to_string();
    let wrong_document = "[]".to_string();

    let cases = [
        (
            "questions_json",
            logic_wasm::parse_logic_text(expected, wrong_questions.clone()),
        ),
        (
            "questions_json",
            logic_wasm::format_logic_text(logic.clone(), wrong_questions.clone()),
        ),
        (
            "questions_json",
            logic_wasm::compile_document_logic(logic.clone(), wrong_questions.clone()),
        ),
        (
            "questions_json",
            logic_wasm::compile_expression(raw.clone(), wrong_questions.clone()),
        ),
        (
            "logic_json",
            logic_wasm::format_logic_text(wrong_logic.clone(), questions.clone()),
        ),
        (
            "logic_json",
            logic_wasm::compile_document_logic(wrong_logic.clone(), questions.clone()),
        ),
        (
            "logic_json",
            logic_wasm::evaluate_document_logic(
                wrong_logic,
                expressions["questions"].to_string(),
                case["answers_json"].as_str().unwrap().to_string(),
            ),
        ),
        (
            "answers_json",
            logic_wasm::evaluate_expression(raw.clone(), questions.clone(), wrong_answers.clone()),
        ),
        (
            "answers_json",
            logic_wasm::evaluate_document_logic(
                case["logic"].to_string(),
                expressions["questions"].to_string(),
                wrong_answers,
            ),
        ),
        (
            "document_json",
            logic_wasm::renew_document_ids(wrong_document),
        ),
    ];
    for (parameter, output) in cases {
        boundary_envelope(&output, parameter);
    }
    // raw_json 的类型是任意 JSON 值，不存在“错误 DTO”；非法 JSON 已由上一条测试覆盖。
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

    // 只有小写连字符 UUID 能跨边界；其余写法在解码阶段拒绝。
    for key in [
        "11111111-1111-4111-8111-11111111111A",
        "11111111111141118111111111111111",
        "00000000-0000-0000-0000-000000000000",
        "不是-uuid",
    ] {
        let answers = format!(r#"{{"{key}":"x"}}"#);
        for output in [
            logic_wasm::evaluate_expression(raw.clone(), questions.clone(), answers.clone()),
            logic_wasm::evaluate_document_logic(logic.clone(), questions.clone(), answers.clone()),
        ] {
            let envelope = boundary_envelope(&output, "answers_json");
            assert_eq!(envelope["error"]["code"], "invalid_syntax");
        }
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
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("规则结构或字段无效"),
        "{error}"
    );

    // 表达式编译错误保留稳定分类与 AST 路径。
    let raw = json!({
        "dsl_version": 2,
        "semantics_version": 1,
        "expression": { "op": "boolean", "value": true },
    })
    .to_string();
    let error = domain_envelope(&logic_wasm::compile_expression(raw, questions_json.clone()));
    assert_eq!(error["code"], "unsupported_version", "{error}");
    assert_eq!(error["path"], "$", "{error}");
    assert!(error["question_id"].is_null(), "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("不支持此规则版本"),
        "{error}"
    );
    assert!(error.get("line").is_none(), "领域错误不带行列：{error}");

    // 文档规则错误保留规则路径与相关题目。
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
    let error = domain_envelope(&logic_wasm::compile_document_logic(
        logic,
        questions_json.clone(),
    ));
    assert_eq!(error["code"], "invalid_reference", "{error}");
    assert_eq!(error["path"], "$.logic.rules[0].visible_if", "{error}");
    assert_eq!(error["question_id"], "00000000-0000-4000-8000-000000000999");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("规则引用的题目或选项无效"),
        "{error}"
    );

    // 表达式求值错误（除零）保留算术分类与根路径。
    let raw = json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "expression": {
            "op": "arithmetic",
            "operator": "divide",
            "left": { "op": "number", "value": "1" },
            "right": { "op": "number", "value": "0" },
        },
    })
    .to_string();
    let error = domain_envelope(&logic_wasm::evaluate_expression(
        raw,
        questions_json.clone(),
        "{}".to_string(),
    ));
    assert_eq!(error["code"], "arithmetic_error", "{error}");
    assert_eq!(error["path"], "$.expression", "{error}");
    assert!(error["question_id"].is_null(), "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("除零或数值溢出"),
        "{error}"
    );

    // 文档求值错误定位到具体计算变量。
    let logic = json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "variables": [{
            "id": "00000000-0000-4000-8000-000000002001",
            "value_type": "number",
            "expression": {
                "op": "arithmetic",
                "operator": "divide",
                "left": { "op": "number", "value": "1" },
                "right": { "op": "number", "value": "0" },
            },
        }],
        "rules": [],
        "flow": null,
    })
    .to_string();
    let error = domain_envelope(&logic_wasm::evaluate_document_logic(
        logic,
        questions_json,
        "{}".to_string(),
    ));
    assert_eq!(error["code"], "arithmetic_error", "{error}");
    assert_eq!(error["path"], "$.logic.variables[0].expression", "{error}");
    assert!(error["question_id"].is_null(), "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("除零或数值溢出"),
        "{error}"
    );
}
