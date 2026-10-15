//! 原生冒烟测试：不走 wasm 目标，直接调用与 `#[wasm_bindgen]` 导出完全相同的
//! 纯函数体（wasm-bindgen 宏在原生目标下编译为普通函数，代码路径不变；
//! `FlowSession` 类导出仅限 `wasm32` 目标）。样例取自 `fixtures/survey-logic/texts.json`
//! 的第一个 `format_cases` 条目，保证与双端回归共用同一语义契约。

use questionnaire_logic::domain::{Answer, Document, FlowEvaluator};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../fixtures/survey-logic/texts.json"))
        .expect("共享样例必须是合法 JSON")
}

/// `format_cases[0]` 的 questions / logic 序列化回文本，供各导出入参复用。
fn sample() -> (String, String) {
    let item = &fixture()["format_cases"][0];
    (
        serde_json::to_string(&item["questions"]).unwrap(),
        serde_json::to_string(&item["logic"]).unwrap(),
    )
}

#[test]
fn 解析文本失败信封携带错误码() {
    let (questions_json, _) = sample();
    let output = logic_wasm::parse_logic_text("@@ 语法错误 @@".to_string(), questions_json);
    let envelope: Value = serde_json::from_str(&output).expect("信封必须是合法 JSON");
    assert!(
        envelope["error"]["code"].is_string(),
        "失败信封缺少 error.code：{envelope}"
    );
}

#[test]
fn 解析共享期望文本成功返回规则包() {
    // format_cases 的 expected 是 TS 真实输出快照，解析它应得到 ok 且包含规则包。
    let fixture = fixture();
    let item = &fixture["format_cases"][0];
    let expected = item["expected"].as_str().unwrap();
    let questions = serde_json::to_string(&item["questions"]).unwrap();
    let output = logic_wasm::parse_logic_text(expected.to_string(), questions);
    let envelope: Value = serde_json::from_str(&output).expect("信封必须是合法 JSON");
    assert!(
        envelope["ok"].is_object(),
        "成功信封应为 ok 对象：{envelope}"
    );
}

#[test]
fn 格式化文本输出与共享期望一致() {
    let (questions_json, logic_json) = sample();
    let output = logic_wasm::format_logic_text(logic_json, questions_json);
    let envelope: Value = serde_json::from_str(&output).expect("信封必须是合法 JSON");
    assert_eq!(envelope["ok"], fixture()["format_cases"][0]["expected"]);
}

#[test]
fn 文档规则预检成功信封可解析() {
    let (questions_json, logic_json) = sample();
    let output = logic_wasm::compile_document_logic(logic_json, questions_json);
    let envelope: Value = serde_json::from_str(&output).expect("信封必须是合法 JSON");
    assert_eq!(envelope["ok"]["compiled"], true);
    assert!(
        envelope["ok"]["variable_order"].is_array(),
        "编译成功应返回变量拓扑顺序：{envelope}"
    );
}

#[test]
fn 表达式编译返回类型与依赖信息() {
    let (questions_json, _) = sample();
    let questions: Value = serde_json::from_str(&questions_json).unwrap();
    // 第 0 题是 single_choice，answer 引用的静态类型是 choice。
    let question_id = questions[0]["id"].as_str().unwrap();
    let raw = serde_json::json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "expression": { "op": "answer", "question_id": question_id }
    });
    let output = logic_wasm::compile_expression(raw.to_string(), questions_json);
    let envelope: Value = serde_json::from_str(&output).expect("信封必须是合法 JSON");
    assert_eq!(envelope["ok"]["value_type"], "choice");
    assert!(
        envelope["ok"]["dependencies"].as_array().is_some(),
        "应返回依赖数组：{envelope}"
    );
}

#[test]
fn 表达式求值返回共享期望的三值结果() {
    let expressions: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/expressions.json"))
            .expect("共享样例必须是合法 JSON");
    // 第 0 题未作答，取反仍是未知：样例同时覆盖 ok 形状与三值语义。
    let case = &expressions["cases"][0];
    let output = logic_wasm::evaluate_expression(
        case["program"].to_string(),
        expressions["questions"].to_string(),
        case["answers_json"].as_str().unwrap().to_string(),
    );
    let envelope: Value = serde_json::from_str(&output).expect("信封必须是合法 JSON");
    assert_eq!(envelope["ok"], case["expected"]["value"], "{envelope}");
}

#[test]
fn 文档规则求值返回变量与规则结果() {
    let expressions: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/expressions.json")).unwrap();
    let documents: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/documents.json")).unwrap();
    // 样例 0 逆序声明两个计算变量；数字按 f64 比较，避免与样例的书写形式较劲。
    let case = &documents["cases"][0];
    let output = logic_wasm::evaluate_document_logic(
        case["logic"].to_string(),
        expressions["questions"].to_string(),
        case["answers_json"].as_str().unwrap().to_string(),
    );
    let envelope: Value = serde_json::from_str(&output).expect("信封必须是合法 JSON");
    let variables = &envelope["ok"]["variables"];
    assert_eq!(
        variables["00000000-0000-4000-8000-000000000201"]["value"].as_f64(),
        Some(2.0)
    );
    assert_eq!(
        variables["00000000-0000-4000-8000-000000000202"]["value"].as_f64(),
        Some(4.0)
    );
    assert_eq!(
        envelope["ok"]["rules"]["00000000-0000-4000-8000-000000000004"]["visible_if"],
        serde_json::json!({ "type": "boolean", "value": true })
    );
}

#[test]
fn 从最终答案重建答卷与共享提交样例一致() {
    let flows: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/flows.json")).unwrap();
    let submissions: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/submissions.json")).unwrap();
    let case = &submissions["cases"][0];
    let document: Document =
        serde_json::from_value(flows["documents"][case["document"].as_str().unwrap()].clone())
            .expect("共享文档必须是合法问卷");
    let answers: Vec<Answer> =
        serde_json::from_str(case["answers_json"].as_str().unwrap()).unwrap();
    // `FlowSession::evaluate_answers` 是 wasm32 限定导出（wasm-bindgen 类 wrapper 原生
    // 会 panic），这里直接走它委托的同一段核心求值，断言同一个 ok 形状。
    let response = FlowEvaluator::new(&document)
        .unwrap()
        .evaluate_answers(answers)
        .expect("共享提交样例应可重建");
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        case["expected"]["value"]
    );
}

#[test]
fn 重写问卷身份后题目标识全部更新() {
    let item = fixture()["format_cases"][0].clone();
    let document = serde_json::json!({
        "title": "冒烟",
        "description": "",
        "purpose": "",
        "thank_you": "",
        "logic": item["logic"],
        "questions": item["questions"],
    });
    let output = logic_wasm::renew_document_ids(document.to_string());
    let envelope: Value = serde_json::from_str(&output).expect("信封必须是合法 JSON");
    let renewed = &envelope["ok"];
    assert!(renewed["questions"][0]["id"].is_string());
    assert_ne!(
        renewed["questions"][0]["id"], item["questions"][0]["id"],
        "题目身份应被重写"
    );
    assert_eq!(
        renewed["logic"]["rules"][0]["question_id"], renewed["questions"][1]["id"],
        "规则引用必须跟随题目身份重写"
    );
    assert!(
        !output.contains(item["questions"][1]["id"].as_str().unwrap()),
        "旧题目身份不应残留在逻辑引用里"
    );
}

// 填写会话仅导出于 wasm32 目标（wasm-bindgen 类 wrapper 原生 panic），
// 该路径由 wasm 侧冒烟脚本（frontend 集成后）覆盖，原生冒烟不重复。
#[cfg(target_arch = "wasm32")]
#[test]
fn 填写会话快照与事件信封可解析() {
    let item = fixture()["format_cases"][0].clone();
    let document = serde_json::json!({
        "title": "冒烟",
        "description": "",
        "purpose": "",
        "thank_you": "",
        "logic": item["logic"],
        "questions": item["questions"],
    });
    let mut session =
        logic_wasm::FlowSession::new(document.to_string()).expect("合法文档应能创建会话");
    let snapshot: Value =
        serde_json::from_str(&session.snapshot()).expect("快照信封必须是合法 JSON");
    assert!(snapshot["ok"]["current_node"].is_string());
    let event = serde_json::json!({
        "type": "change",
        "question_id": item["questions"][0]["id"],
        "value": item["questions"][0]["options"][0]["id"],
    });
    let dispatched: Value = serde_json::from_str(&session.dispatch(event.to_string()))
        .expect("事件信封必须是合法 JSON");
    // 事件可能因流程结构被拒绝，但无论 ok 还是 error 都必须是可解析信封。
    assert!(
        dispatched.get("ok").is_some() || dispatched["error"]["code"].is_string(),
        "事件结果应为信封：{dispatched}"
    );
    // 非法事件 JSON 必须给出 error.code。
    let invalid: Value = serde_json::from_str(&session.dispatch("{ 坏 JSON".to_string())).unwrap();
    assert!(invalid["error"]["code"].is_string(), "{invalid}");
    // 非法文档必须以 Err(JSON 字符串) 拒绝。
    let Err(err) = logic_wasm::FlowSession::new("不是 JSON".to_string()) else {
        panic!("非法文档必须返回错误");
    };
    let envelope: Value = serde_json::from_str(&err.as_string().unwrap()).unwrap();
    assert!(envelope["error"]["code"].is_string(), "{envelope}");
}
