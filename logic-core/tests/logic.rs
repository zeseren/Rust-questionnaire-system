//! 逻辑编译回归：共享样例、版本门禁、资源预算与编译结果契约。

use std::collections::BTreeMap;

use questionnaire_logic::domain::{
    Choice, DocumentLogic, LogicErrorCode, LogicValue, LogicValueType, Question, QuestionKind,
    compile_expression,
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

fn fixtures() -> Fixtures {
    serde_json::from_str(include_str!("../../fixtures/survey-logic/expressions.json")).unwrap()
}

fn documents() -> Value {
    serde_json::from_str(include_str!("../../fixtures/survey-logic/documents.json")).unwrap()
}

#[test]
fn 表达式共享样例全部保持编译与三值求值语义() {
    let fixtures = fixtures();
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
fn 文档共享样例保持变量排序和间接依赖规则一致() {
    let fixtures = fixtures();
    let documents = documents();
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
            assert_eq!(error.code, LogicErrorCode::LimitExceeded);
            assert_eq!(error.path, "$.expression");
            assert_eq!(error.question_id, None);
        }
    }
}

#[test]
fn 版本号只接受整数_1() {
    let cases = [
        (
            "表达式 DSL 版本过新",
            json!(2),
            json!(1),
            LogicErrorCode::UnsupportedVersion,
        ),
        (
            "表达式语义版本过新",
            json!(1),
            json!(2),
            LogicErrorCode::UnsupportedVersion,
        ),
        (
            "表达式语义版本过旧",
            json!(1),
            json!(0),
            LogicErrorCode::UnsupportedVersion,
        ),
        (
            "非整数版本",
            json!(1.5),
            json!(1),
            LogicErrorCode::InvalidSyntax,
        ),
    ];
    for (name, dsl_version, semantics_version, expected) in cases {
        let raw = json!({
            "dsl_version": dsl_version.clone(),
            "semantics_version": semantics_version.clone(),
            "expression": { "op": "boolean", "value": true },
        });
        let error = compile_expression(&raw, &[]).unwrap_err();
        assert_eq!(error.code, expected, "{name}（表达式）");
        assert_eq!(error.path, "$", "{name}（表达式路径）");

        let logic: DocumentLogic = serde_json::from_value(json!({
            "dsl_version": dsl_version,
            "semantics_version": semantics_version,
            "variables": [],
            "rules": [],
            "flow": null,
        }))
        .expect("版本字段仍是合法数字");
        let error = logic.compile(&[]).unwrap_err();
        assert_eq!(error.code, expected, "{name}（规则包）");
        assert_eq!(error.path, "$", "{name}（规则包路径）");
    }

    // 字符串版本连 DTO 都无法构成；两条入口都必须在最外层拒绝。
    let raw = json!({
        "dsl_version": "1",
        "semantics_version": 1,
        "expression": { "op": "boolean", "value": true },
    });
    assert_eq!(
        compile_expression(&raw, &[]).unwrap_err().code,
        LogicErrorCode::InvalidSyntax
    );
    let error = serde_json::from_value::<DocumentLogic>(json!({
        "dsl_version": "1",
        "semantics_version": 1,
        "variables": [],
        "rules": [],
        "flow": null,
    }))
    .unwrap_err();
    assert!(
        error.to_string().contains("invalid type: string"),
        "版本字段类型必须严格：{error}"
    );
}

#[test]
fn 编译结果公开类型依赖与求值契约() {
    let fixtures = fixtures();
    let text_question = fixtures.questions[2].id;
    let number_question = fixtures.questions[3].id;
    let program = json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "expression": {
            "op": "all",
            "expressions": [
                {
                    "op": "compare",
                    "comparison": "gt",
                    "left": {
                        "op": "length",
                        "value": { "op": "answer", "question_id": text_question },
                    },
                    "right": { "op": "number", "value": "0" },
                },
                {
                    "op": "compare",
                    "comparison": "lt",
                    "left": { "op": "answer", "question_id": number_question },
                    "right": { "op": "number", "value": "10" },
                },
                { "op": "answered", "question_id": text_question },
            ],
        },
    });
    let compiled = compile_expression(&program, &fixtures.questions).unwrap();
    assert_eq!(compiled.value_type(), LogicValueType::Boolean);
    // 同一题被引用两次只保留一个依赖，且按 UUID 排序。
    assert_eq!(
        compiled.dependencies().collect::<Vec<_>>(),
        vec![text_question, number_question]
    );

    let mut answers = BTreeMap::new();
    answers.insert(text_question, json!("文本"));
    answers.insert(number_question, json!(2));
    assert_eq!(
        compiled.evaluate(&answers).unwrap(),
        LogicValue::Boolean(true)
    );

    // 数字题缺失让短路后的合取保持未知，而不是当作零。
    answers.remove(&number_question);
    assert_eq!(compiled.evaluate(&answers).unwrap(), LogicValue::Missing);

    // 文本题缺失但 answered 为假时，合取已有明确假值。
    answers.insert(number_question, json!(2));
    answers.remove(&text_question);
    assert_eq!(
        compiled.evaluate(&answers).unwrap(),
        LogicValue::Boolean(false)
    );
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
        assert_eq!(error.code, LogicErrorCode::InvalidReference, "{name}");
    }
}

#[test]
fn 三元条件短路未命中分支且负号保持有限数值契约() {
    let fixtures = fixtures();
    let questions = &fixtures.questions;
    let base = |expression: Value| {
        json!({
            "dsl_version": 1,
            "semantics_version": 1,
            "expression": expression,
        })
    };

    // 条件为 false 时只求值 else，then 分支里的除零不得被发现。
    let compiled = compile_expression(
        &base(json!({
            "op": "if",
            "condition": { "op": "boolean", "value": false },
            "then_branch": {
                "op": "arithmetic", "operator": "divide",
                "left": { "op": "number", "value": "1" },
                "right": { "op": "number", "value": "0" },
            },
            "else_branch": {
                "op": "arithmetic", "operator": "multiply",
                "left": { "op": "number", "value": "2" },
                "right": { "op": "number", "value": "3" },
            },
        })),
        questions,
    )
    .unwrap();
    assert_eq!(compiled.value_type(), LogicValueType::Number);
    assert_eq!(
        compiled.evaluate(&BTreeMap::new()).unwrap(),
        LogicValue::Number(6.0)
    );

    // 条件缺失时三元短路为 Missing，不读取任何分支。
    let compiled = compile_expression(
        &base(json!({
            "op": "if",
            "condition": {
                "op": "compare",
                "comparison": "gt",
                "left": { "op": "answer", "question_id": fixtures.questions[3].id },
                "right": { "op": "number", "value": "0" },
            },
            "then_branch": { "op": "number", "value": "1" },
            "else_branch": { "op": "number", "value": "0" },
        })),
        questions,
    )
    .unwrap();
    assert_eq!(
        compiled.evaluate(&BTreeMap::new()).unwrap(),
        LogicValue::Missing
    );

    // 负号归一正零，并拒绝非数字操作数。
    let compiled = compile_expression(
        &base(json!({ "op": "negate", "value": { "op": "number", "value": "-0" } })),
        questions,
    )
    .unwrap();
    assert_eq!(
        compiled.evaluate(&BTreeMap::new()).unwrap(),
        LogicValue::Number(0.0)
    );
    let error = compile_expression(
        &base(json!({ "op": "negate", "value": { "op": "text", "value": "1" } })),
        questions,
    )
    .unwrap_err();
    assert_eq!(error.code, LogicErrorCode::TypeMismatch);
}

#[test]
fn 文本函数与单选相等比较遵循静态类型() {
    let fixtures = fixtures();
    let questions = &fixtures.questions;
    let base = |expression: Value| {
        json!({
            "dsl_version": 1,
            "semantics_version": 1,
            "expression": expression,
        })
    };
    let text_question = fixtures.questions[2].id;
    let choice_question = fixtures.questions[0].id;

    let compiled = compile_expression(
        &base(json!({
            "op": "all",
            "expressions": [
                {
                    "op": "contains",
                    "text": { "op": "answer", "question_id": text_question },
                    "substring": { "op": "text", "value": "馈" },
                },
                {
                    "op": "starts_with",
                    "text": { "op": "answer", "question_id": text_question },
                    "prefix": { "op": "text", "value": "反" },
                },
                {
                    "op": "compare",
                    "comparison": "eq",
                    "left": { "op": "answer", "question_id": choice_question },
                    "right": { "op": "answer", "question_id": choice_question },
                },
            ],
        })),
        questions,
    )
    .unwrap();
    let mut answers = BTreeMap::new();
    answers.insert(text_question, json!("反馈"));
    answers.insert(
        choice_question,
        json!(fixtures.questions[0].kind.options().unwrap()[0].id),
    );
    assert_eq!(
        compiled.evaluate(&answers).unwrap(),
        LogicValue::Boolean(true)
    );

    // 文本函数不接受数字操作数；选择身份不允许大小排序。
    let error = compile_expression(
        &base(json!({
            "op": "contains",
            "text": { "op": "number", "value": "1" },
            "substring": { "op": "text", "value": "1" },
        })),
        questions,
    )
    .unwrap_err();
    assert_eq!(error.code, LogicErrorCode::TypeMismatch);

    let error = compile_expression(
        &base(json!({
            "op": "compare",
            "comparison": "lt",
            "left": { "op": "answer", "question_id": choice_question },
            "right": { "op": "answer", "question_id": choice_question },
        })),
        questions,
    )
    .unwrap_err();
    assert_eq!(error.code, LogicErrorCode::TypeMismatch);

    // 缺失的选择身份比较保持 Missing。
    let compiled = compile_expression(
        &base(json!({
            "op": "compare",
            "comparison": "ne",
            "left": { "op": "answer", "question_id": choice_question },
            "right": { "op": "answer", "question_id": choice_question },
        })),
        questions,
    )
    .unwrap();
    assert_eq!(
        compiled.evaluate(&BTreeMap::new()).unwrap(),
        LogicValue::Missing
    );
}
