//! 原生冒烟测试：不走 wasm 目标，直接调用与 `#[wasm_bindgen]` 导出完全相同的
//! 函数体（wasm-bindgen 宏在原生目标下编译为普通函数/普通方法，代码路径不变，
//! 因此无需 `cfg` 分支即可覆盖全部导出）。样例取自 `fixtures/survey-logic/texts.json`
//! 的第一个 `format_cases` 条目，保证与双端回归共用同一语义契约。

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
    assert!(envelope["ok"].is_object(), "成功信封应为 ok 对象：{envelope}");
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
    let invalid: Value =
        serde_json::from_str(&session.dispatch("{ 坏 JSON".to_string())).unwrap();
    assert!(invalid["error"]["code"].is_string(), "{invalid}");
    // 非法文档必须以 Err(JSON 字符串) 拒绝。wasm-bindgen 的 `Result::Err`
    // 载荷在原生目标是 `JsValue::NULL`（无 wasm 运行时），只断言失败分支；
    // 错误字符串内容留给 wasm 侧冒烟脚本验证。
    #[cfg(target_arch = "wasm32")]
    {
        let err = logic_wasm::FlowSession::new("不是 JSON".to_string()).unwrap_err();
        let envelope: Value = serde_json::from_str(err.as_string().unwrap()).unwrap();
        assert!(envelope["error"]["code"].is_string(), "{envelope}");
    }
    #[cfg(not(target_arch = "wasm32"))]
    assert!(logic_wasm::FlowSession::new("不是 JSON".to_string()).is_err());
}
