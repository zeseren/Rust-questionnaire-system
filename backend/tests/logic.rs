//! 与浏览器共用语义样例，防止缺失值、数值边界、错误分类和资源预算发生漂移。
use std::collections::BTreeMap;

use questionnaire_api::domain::{Question, compile_expression};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Deserialize)]
struct Fixtures {
    questions: Vec<Question>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    program: Value,
    answers_json: String,
    expected: Value,
}

#[test]
fn 两端共享样例保持编译与求值语义一致() {
    let fixtures: Fixtures =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/expressions.json")).unwrap();
    for case in fixtures.cases {
        let answers: BTreeMap<Uuid, Value> = serde_json::from_str(&case.answers_json).unwrap();
        let result = compile_expression(&case.program, &fixtures.questions)
            .and_then(|compiled| compiled.evaluate(&answers));
        let actual = match result {
            Ok(value) => json!({ "value": value }),
            Err(error) => json!({ "error": {
                "code": error.code, "path": error.path, "question_id": error.question_id,
            }}),
        };
        let mut expected = case.expected;
        // JSON 的 7 与 7.0 是同一数值；只统一结果表示，不放宽比较精度。
        if expected["value"]["type"] == "number" {
            let number = expected["value"]["value"].as_f64().unwrap();
            expected["value"]["value"] = json!(number);
        }
        assert_eq!(actual, expected, "语义样例：{}", case.name);
    }
}

#[test]
fn 深度和节点上限在编译之前拦截且允许边界() {
    for (depth, count, allowed) in [
        (16, 1, true),
        (17, 1, false),
        (2, 256, true),
        (2, 257, false),
    ] {
        let mut expression = json!({ "op": "boolean", "value": true });
        if count > 1 {
            expression = json!({ "op": "all", "expressions": vec![expression; count - 1] });
        } else {
            for _ in 1..depth {
                expression = json!({ "op": "not", "expression": expression });
            }
        }
        let program = json!({ "dsl_version": 1, "semantics_version": 1, "expression": expression });
        let result = compile_expression(&program, &[]);
        if allowed {
            assert!(result.is_ok(), "应允许深度 {depth}、节点数 {count} 的边界");
        } else {
            let error = result.unwrap_err();
            assert_eq!(serde_json::to_value(error.code).unwrap(), "limit_exceeded");
        }
    }
}

#[test]
fn 编译结果保留静态类型和去重依赖且可重复求值() {
    let fixtures: Fixtures =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/expressions.json")).unwrap();
    let id = fixtures.questions[0].id;
    let program = json!({ "dsl_version": 1, "semantics_version": 1, "expression": {
        "op": "all", "expressions": [
            { "op": "answered", "question_id": id },
            { "op": "answered", "question_id": id },
        ]
    }});
    let compiled = compile_expression(&program, &fixtures.questions).unwrap();
    assert_eq!(compiled.dependencies().collect::<Vec<_>>(), vec![id]);
    assert_eq!(
        serde_json::to_value(compiled.value_type()).unwrap(),
        "boolean"
    );
    let empty = BTreeMap::new();
    assert_eq!(
        serde_json::to_value(compiled.evaluate(&empty).unwrap()).unwrap(),
        json!({"type": "boolean", "value": false})
    );
    let answers = BTreeMap::from([(id, json!("00000000-0000-4000-8000-000000000101"))]);
    assert_eq!(
        serde_json::to_value(compiled.evaluate(&answers).unwrap()).unwrap(),
        json!({"type": "boolean", "value": true})
    );
    assert_eq!(
        serde_json::to_value(compiled.evaluate(&empty).unwrap()).unwrap(),
        json!({"type": "boolean", "value": false})
    );
}
