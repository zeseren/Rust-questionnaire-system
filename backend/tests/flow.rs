//! 两端执行同一份流程和事件序列，防止路径、清理、回退与缺失处理发生漂移。
use questionnaire_api::domain::{
    Answer, Document, FlowDefinition, FlowEvaluator, FlowEvent, FlowNode, FlowOutcome, FlowSession,
    QuestionKind,
};
use serde_json::{Value, json};
use uuid::Uuid;

fn fixtures() -> Value {
    serde_json::from_str(include_str!("../../fixtures/survey-logic/flows.json")).unwrap()
}

#[test]
fn 配置与作答题目身份拒绝非规范_uuid且选项值仍规范化() {
    let contracts: Value = serde_json::from_str(include_str!(
        "../../fixtures/survey-logic/input-contracts.json"
    ))
    .unwrap();
    let source = &fixtures()["documents"][contracts["document"].as_str().unwrap()];
    let canonical = contracts["canonical_id"].as_str().unwrap();
    for path in contracts["configuration_paths"].as_array().unwrap() {
        let path = path.as_str().unwrap();
        let original = source.pointer(path).unwrap().as_str().unwrap();
        // 先同步替换声明和引用，保证标准写法仍构成有效问卷；再单独扰动待测字段。
        let renamed = serde_json::to_string(source)
            .unwrap()
            .replace(original, canonical);
        for case in contracts["uuid_cases"].as_array().unwrap() {
            let mut raw: Value = serde_json::from_str(&renamed).unwrap();
            *raw.pointer_mut(path).unwrap() = case["raw"].clone();
            let valid = serde_json::from_value::<Document>(raw)
                .is_ok_and(|document| document.compile_logic().is_ok());
            assert_eq!(
                valid,
                case["configuration_valid"].as_bool().unwrap(),
                "{path}：{}",
                case["name"]
            );
        }
    }
    let mut raw = source.clone();
    raw.as_object_mut().unwrap().remove("logic");
    raw["questions"].as_array_mut().unwrap().truncate(1);
    raw["questions"][0]["options"][0]["id"] = canonical.into();
    let document: Document = serde_json::from_value(raw.clone()).unwrap();
    let question_id = document.questions[0].id;
    for case in contracts["uuid_cases"].as_array().unwrap() {
        let result = document.validate_answers(vec![Answer {
            question_id,
            value: case["raw"].clone(),
        }]);
        assert_eq!(
            result.is_ok(),
            case["option_valid"].as_bool().unwrap(),
            "选项答案：{}",
            case["name"]
        );
        if let Ok(answers) = result {
            assert_eq!(answers[&question_id], canonical);
        }
    }
    raw["questions"][0] = source["questions"][0].clone();
    raw["questions"][0]["id"] = canonical.into();
    let document: Document = serde_json::from_value(raw).unwrap();
    let value = &source["questions"][0]["options"][0]["id"];
    for case in contracts["uuid_cases"].as_array().unwrap() {
        let valid = case["configuration_valid"].as_bool().unwrap();
        let answer = json!({ "question_id": case["raw"], "value": value });
        let accepted = serde_json::from_value::<Answer>(answer)
            .is_ok_and(|answer| document.validate_answers(vec![answer]).is_ok());
        assert_eq!(accepted, valid, "答案题目身份：{}", case["name"]);
        let event = json!({ "type": "change", "question_id": case["raw"], "value": value });
        let mut session = FlowSession::new(&document).unwrap();
        let accepted = serde_json::from_value::<FlowEvent>(event)
            .is_ok_and(|event| session.dispatch(event).is_ok());
        assert_eq!(accepted, valid, "事件题目身份：{}", case["name"]);
    }
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

    // 257 个节点超过 256 上限；用循环构造，替代手写的大流程样例。
    let mut document: Document =
        serde_json::from_value(fixtures["documents"]["diamond"].clone()).unwrap();
    let nodes = (0..257)
        .map(|index| FlowNode::End {
            id: Uuid::from_u128(1_000 + index),
            outcome: FlowOutcome::Complete,
        })
        .collect();
    document.logic.as_mut().unwrap().flow = Some(FlowDefinition {
        entry: Uuid::from_u128(1_000),
        nodes,
    });
    let failure = FlowEvaluator::new(&document).unwrap_err();
    assert_eq!(json!(failure.code), "limit_exceeded");
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
fn 整卷复制重写配置及引用且仅普通完成流程可发布() {
    let fixtures = fixtures();
    let source: Document =
        serde_json::from_value(fixtures["documents"]["diamond"].clone()).unwrap();
    let mut copied = source.clone();
    copied.renew_ids();
    copied.validate_publish().unwrap();
    let screened: Document =
        serde_json::from_value(fixtures["documents"]["priority"].clone()).unwrap();
    assert!(screened.validate_publish().is_err());
    let source_flow = source.logic.as_ref().unwrap().flow.as_ref().unwrap();
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
        if let QuestionKind::SingleChoice { options }
        | QuestionKind::MultipleChoice { options, .. } = &question.kind
        {
            for option in options {
                assert!(
                    !copied_json.contains(&option.id.to_string()),
                    "复制不能遗留来源选项身份"
                );
            }
        }
    }
    for variable in &source.logic.as_ref().unwrap().variables {
        assert!(
            !copied_json.contains(&variable.id.to_string()),
            "复制不能遗留来源变量身份"
        );
    }
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
