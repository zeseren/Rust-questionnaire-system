//! 与浏览器共用语义样例，防止缺失值、数值边界、错误分类和资源预算发生漂移。
use std::collections::BTreeMap;

use questionnaire_api::domain::{
    Choice, DocumentLogic, Question, QuestionKind, compile_expression,
};
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
fn 空规则包编译仍拦截重复或全零的题目与选项身份() {
    // 空规则包同样绑定题目身份，不能因没有表达式而跳过题目/选项唯一性检查。
    let empty: DocumentLogic = serde_json::from_value(json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "variables": [],
        "rules": [],
        "flow": null,
    }))
    .expect("空规则包必须是合法的文档规则");

    let text = |id: Uuid| Question {
        id,
        title: "题".into(),
        required: false,
        kind: QuestionKind::Text { max_length: 20 },
    };
    let duplicate_question = Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1111);
    let duplicate_option = Uuid::from_u128(0x2222_2222_2222_4222_8222_2222_2222_2222);
    let cases = [
        ("全零题目身份", vec![text(Uuid::nil())]),
        (
            "重复题目身份",
            vec![text(duplicate_question), text(duplicate_question)],
        ),
        (
            "重复选项身份",
            vec![Question {
                id: duplicate_question,
                title: "单选".into(),
                required: false,
                kind: QuestionKind::SingleChoice {
                    options: vec![
                        Choice {
                            id: duplicate_option,
                            label: "甲".into(),
                        },
                        Choice {
                            id: duplicate_option,
                            label: "乙".into(),
                        },
                    ],
                },
            }],
        ),
    ];

    for (name, questions) in cases {
        let error = empty.compile(&questions).unwrap_err();
        assert_eq!(
            serde_json::to_value(error.code).unwrap(),
            "invalid_reference",
            "{name} 必须在编译前被拒绝"
        );
    }
}
