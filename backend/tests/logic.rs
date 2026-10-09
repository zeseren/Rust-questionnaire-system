//! 与浏览器共用语义样例，防止缺失值、数值边界、错误分类和资源预算发生漂移。
use std::collections::BTreeMap;

use questionnaire_api::domain::{DocumentLogic, Question, compile_expression};
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
fn 文档共享样例保持变量排序和间接依赖规则一致() {
    let fixtures: Fixtures =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/expressions.json")).unwrap();
    let documents: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/documents.json")).unwrap();
    for case in documents["cases"].as_array().unwrap() {
        let logic: DocumentLogic = serde_json::from_value(case["logic"].clone()).unwrap();
        let answers: BTreeMap<Uuid, Value> =
            serde_json::from_str(case["answers_json"].as_str().unwrap()).unwrap();
        let result = logic.compile(&fixtures.questions).and_then(|compiled| {
            compiled
                .evaluate(&answers)
                .map(|value| json!({ "order": compiled.variable_order(), "value": value }))
        });
        let actual = result.unwrap_or_else(
            |error| json!({"error": {"code": error.code, "question_id": error.question_id}}),
        );
        let mut expected = case["expected"].clone();
        normalize_expected_numbers(&mut expected);
        assert_eq!(actual, expected, "文档语义样例：{}", case["name"]);
    }
}

fn normalize_expected_numbers(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            if fields.get("type").is_some_and(|value| value == "number") {
                let number = fields["value"].as_f64().unwrap();
                fields.insert("value".into(), json!(number));
            } else {
                for value in fields.values_mut() {
                    normalize_expected_numbers(value);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_expected_numbers(value);
            }
        }
        _ => {}
    }
}

#[test]
fn 文档总预算不能被拆成多个合法小表达式绕过() {
    let expression =
        json!({"op": "all", "expressions": vec![json!({"op": "boolean", "value": true}); 255]});
    let variables: Vec<_> = (1..=17).map(|id| json!({"id": Uuid::from_u128(id), "value_type": "boolean", "expression": expression})).collect();
    let raw =
        json!({"dsl_version": 1, "semantics_version": 1, "variables": variables, "rules": []});
    let mut logic: DocumentLogic = serde_json::from_value(raw).unwrap();
    assert_eq!(
        serde_json::to_value(logic.compile(&[]).unwrap_err().code).unwrap(),
        "limit_exceeded"
    );
    logic.variables.pop();
    assert!(logic.compile(&[]).is_ok());
}
