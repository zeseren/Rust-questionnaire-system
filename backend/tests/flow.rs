//! 两端执行同一份流程和事件序列，防止路径、清理、回退与缺失处理发生漂移。
use questionnaire_api::domain::{
    Answer, Document, FlowEvaluator, FlowEvent, FlowNode, FlowSession,
};
use serde_json::{Value, json};

fn fixtures() -> Value {
    serde_json::from_str(include_str!("../../fixtures/survey-logic/flows.json")).unwrap()
}

#[test]
fn 共享流程拒绝非法拓扑和未保护的分支依赖() {
    let fixtures = fixtures();
    for case in fixtures["compilation"].as_array().unwrap() {
        let mut document = fixtures["documents"][case["document"].as_str().unwrap()].clone();
        for edit in case["edits"].as_array().unwrap() {
            let mut target = &mut document;
            for segment in edit["path"].as_array().unwrap() {
                target = match segment {
                    Value::String(key) => &mut target[key],
                    Value::Number(index) => &mut target[index.as_u64().unwrap() as usize],
                    _ => panic!("样例路径只能包含字段或索引"),
                };
            }
            *target = edit["value"].clone();
        }
        let document: Document = serde_json::from_value(document).unwrap();
        let actual = document.compile_logic().err().map(|failure| failure.code);
        assert_eq!(json!(actual), case["error"], "编译样例：{}", case["name"]);
    }
}

#[test]
fn 共享事件序列保持路径清理回退和题型规范化一致() {
    let fixtures = fixtures();
    for case in fixtures["sessions"].as_array().unwrap() {
        let document: Document = serde_json::from_value(
            fixtures["documents"][case["document"].as_str().unwrap()].clone(),
        )
        .unwrap();
        let mut session = FlowSession::new(&document).unwrap();
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            let before = serde_json::to_value(session.snapshot()).unwrap();
            if let Some(event) = step["event_json"].as_str() {
                let event: FlowEvent = serde_json::from_str(event).unwrap();
                let actual = session.dispatch(event).err().map(|failure| failure.code);
                assert_eq!(
                    json!(actual),
                    step["error"],
                    "事件样例：{}，第 {index} 步",
                    case["name"]
                );
                if actual.is_some() {
                    assert_eq!(
                        serde_json::to_value(session.snapshot()).unwrap(),
                        before,
                        "失败事件不能改变状态"
                    );
                }
            }
            let actual = serde_json::to_value(session.snapshot()).unwrap();
            for (key, expected) in step["expected"].as_object().unwrap() {
                assert!(
                    equivalent(&actual[key], expected),
                    "快照样例：{}，第 {index} 步，{key}：实际 {}，预期 {expected}",
                    case["name"],
                    actual[key]
                );
            }
        }
    }
}

// JSON 的整数与浮点表示不影响数值语义，其他字段仍严格比较。
fn equivalent(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && b.iter()
                    .all(|(key, value)| a.get(key).is_some_and(|actual| equivalent(actual, value)))
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equivalent(a, b))
        }
        _ => actual == expected,
    }
}

#[test]
fn 整卷复制重写流程所有身份且仅普通完成流程可发布() {
    let fixtures = fixtures();
    let source: Document =
        serde_json::from_value(fixtures["documents"]["diamond"].clone()).unwrap();
    let mut copied = source.clone();
    copied.renew_ids();
    assert!(copied.validate_draft().is_ok());
    source.validate_publish().unwrap();
    copied.validate_publish().unwrap();
    let screened: Document =
        serde_json::from_value(fixtures["documents"]["priority"].clone()).unwrap();
    assert!(screened.validate_publish().is_err());
    let source_flow = source.logic.as_ref().unwrap().flow.as_ref().unwrap();
    let copied_flow = copied.logic.as_ref().unwrap().flow.as_ref().unwrap();
    assert_ne!(source_flow.entry, copied_flow.entry);
    let copied_json = serde_json::to_string(&copied).unwrap();
    for node in &source_flow.nodes {
        let id = match node {
            FlowNode::Page { id, .. } | FlowNode::Branch { id, .. } | FlowNode::End { id, .. } => {
                id
            }
        };
        assert!(
            !copied_json.contains(&id.to_string()),
            "复制不能遗留来源节点身份"
        );
    }
    for question in &source.questions {
        assert!(
            !copied_json.contains(&question.id.to_string()),
            "复制不能遗留来源题目引用"
        );
    }
    let session = FlowSession::new(&copied).unwrap();
    assert_eq!(session.snapshot().current_node, copied_flow.entry);
}

#[test]
fn 最终提交共享样例拒绝路径外输入并重建适用性() {
    let fixtures = fixtures();
    let submissions: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/submissions.json")).unwrap();
    for case in submissions["cases"].as_array().unwrap() {
        let document: Document = serde_json::from_value(
            fixtures["documents"][case["document"].as_str().unwrap()].clone(),
        )
        .unwrap();
        let answers: Vec<Answer> =
            serde_json::from_str(case["answers_json"].as_str().unwrap()).unwrap();
        let result = FlowEvaluator::new(&document)
            .unwrap()
            .evaluate_answers(answers.clone());
        let actual = match result {
            Ok(value) => {
                // 同时守住 HTTP 实际调用的领域入口，避免只测纯引擎却遗漏提交接线。
                assert_eq!(document.validate_answers(answers).unwrap(), value.answers);
                json!({"value": value})
            }
            Err(error) => {
                assert!(document.validate_answers(answers).is_err());
                json!({"error": {"code": error.code, "question_id": error.question_id}})
            }
        };
        assert!(
            equivalent(&actual, &case["expected"]),
            "提交样例：{}，实际 {actual}，预期 {}",
            case["name"],
            case["expected"]
        );
    }
}
