//! 领域回归守住当前问卷的身份、答案规范化与实际提交容量。
use questionnaire_api::domain::{
    Answer, Choice, Document, DocumentLogic, FlowEvaluator, LogicErrorCode, MAX_NUMBER_JSON_BYTES,
    MAX_QUESTIONS, MAX_SUBMISSION_BYTES, MAX_TEXT_ANSWER_LENGTH, MAX_TEXT_LENGTH,
    MAX_TEXTAREA_ANSWER_LENGTH, Question, QuestionKind,
};
use serde_json::{Value, json};
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn question(value: u128, required: bool, kind: QuestionKind) -> Question {
    Question {
        id: id(value),
        title: "题目".into(),
        required,
        kind,
    }
}

fn choices() -> Vec<Choice> {
    vec![
        Choice {
            id: id(101),
            label: "选项甲".into(),
        },
        Choice {
            id: id(102),
            label: "选项乙".into(),
        },
    ]
}

fn document(questions: Vec<Question>) -> Document {
    Document {
        logic: None,
        title: "课程反馈".into(),
        description: String::new(),
        purpose: "用于改进课程安排".into(),
        thank_you: String::new(),
        questions,
    }
}

fn answer(question: u128, value: Value) -> Answer {
    Answer {
        question_id: id(question),
        value,
    }
}

#[test]
fn 共享空白样例保持普通答卷与流程的必填和省略语义一致() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../fixtures/survey-logic/input-contracts.json"
    ))
    .unwrap();
    for case in fixtures["text_cases"].as_array().unwrap() {
        let answered = case["answered"].as_bool().unwrap();
        for required in [false, true] {
            let doc = document(vec![question(
                1,
                required,
                QuestionKind::Textarea { max_length: 1000 },
            )]);
            let answers = vec![answer(1, case["value"].clone())];
            let plain = doc.validate_answers(answers.clone());
            let flow = questionnaire_api::domain::FlowEvaluator::new(&doc)
                .unwrap()
                .evaluate_answers(answers);
            if required && !answered {
                assert_eq!(
                    plain.unwrap_err().question_id,
                    Some(id(1)),
                    "{}",
                    case["name"]
                );
                assert_eq!(
                    flow.unwrap_err().question_id,
                    Some(id(1)),
                    "{}",
                    case["name"]
                );
            } else {
                let plain = plain.unwrap();
                assert_eq!(plain, flow.unwrap().answers, "{}", case["name"]);
                assert_eq!(
                    plain.get(&id(1)),
                    answered.then_some(&case["value"]),
                    "{}",
                    case["name"]
                );
            }
        }
    }
}

#[test]
fn 答案顺序选项顺序和数值写法不影响规范化结果() {
    let doc = document(vec![
        question(
            1,
            true,
            QuestionKind::MultipleChoice {
                options: choices(),
                min_choices: 1,
                max_choices: 2,
            },
        ),
        question(
            2,
            true,
            QuestionKind::Number {
                integer: false,
                min: None,
                max: None,
            },
        ),
        question(3, true, QuestionKind::Textarea { max_length: 50 }),
        question(
            4,
            true,
            QuestionKind::Rating {
                min: 0,
                max: 5,
                min_label: String::new(),
                max_label: String::new(),
            },
        ),
    ]);
    let first = doc
        .validate_answers(vec![
            answer(1, json!([id(102), id(101)])),
            answer(2, json!(1)),
            answer(3, json!("  原文\n保留  ")),
            answer(4, json!(-0.0)),
        ])
        .unwrap();
    let second = doc
        .validate_answers(vec![
            answer(4, json!(0)),
            answer(3, json!("  原文\n保留  ")),
            answer(2, json!(1.0)),
            answer(1, json!([id(101).simple().to_string(), id(102)])),
        ])
        .unwrap();

    // 幂等指纹使用序列化结果；仅比较 Value 可能漏掉负零等表现形式差异。
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(first[&id(1)], json!([id(101), id(102)]));
    assert_eq!(first[&id(3)], json!("  原文\n保留  "));
}

#[test]
fn 重复和外来身份不能通过覆盖绕过校验() {
    let mut doc = document(vec![question(
        1,
        false,
        QuestionKind::MultipleChoice {
            options: choices(),
            min_choices: 1,
            max_choices: 2,
        },
    )]);
    let duplicate_question = doc
        .validate_answers(vec![answer(1, Value::Null), answer(1, json!([id(101)]))])
        .unwrap_err();
    assert_eq!(duplicate_question.question_id, Some(id(1)));
    let duplicate_choice = doc
        .validate_answers(vec![answer(
            1,
            json!([id(101), id(101).simple().to_string()]),
        )])
        .unwrap_err();
    assert_eq!(duplicate_choice.question_id, Some(id(1)));
    let foreign_question = doc
        .validate_answers(vec![answer(99, Value::Null)])
        .unwrap_err();
    assert_eq!(foreign_question.question_id, Some(id(99)));
    let foreign_choice = doc
        .validate_answers(vec![answer(1, json!([id(999)]))])
        .unwrap_err();
    assert_eq!(foreign_choice.question_id, Some(id(1)));

    doc.questions.push(doc.questions[0].clone());
    assert_eq!(doc.validate_draft().unwrap_err().question_id, Some(id(1)));
    doc.questions[1].id = id(2);
    // 第二题身份改正后，复制来的选项身份仍然重复，不能静默修复。
    assert_eq!(doc.validate_draft().unwrap_err().question_id, Some(id(2)));
}

#[test]
fn 可选空答案省略而零仍是有效必填答案() {
    let doc = document(vec![
        question(
            1,
            true,
            QuestionKind::Rating {
                min: 0,
                max: 5,
                min_label: String::new(),
                max_label: String::new(),
            },
        ),
        question(2, false, QuestionKind::Text { max_length: 20 }),
        question(
            3,
            false,
            QuestionKind::MultipleChoice {
                options: choices(),
                min_choices: 1,
                max_choices: 2,
            },
        ),
        question(
            4,
            false,
            QuestionKind::Number {
                integer: true,
                min: Some(0.0),
                max: None,
            },
        ),
    ]);
    let missing = doc.validate_answers(Vec::new()).unwrap_err();
    assert_eq!(missing.question_id, Some(id(1)));
    let null = doc
        .validate_answers(vec![answer(1, Value::Null)])
        .unwrap_err();
    assert_eq!(null.question_id, Some(id(1)));
    let normalized = doc
        .validate_answers(vec![
            answer(1, json!(0)),
            answer(2, json!(" \u{3000} ")),
            answer(3, json!([])),
            answer(4, Value::Null),
        ])
        .unwrap();
    assert_eq!(normalized.keys().copied().collect::<Vec<_>>(), vec![id(1)]);
    assert_eq!(normalized[&id(1)].as_f64(), Some(0.0));
}

#[test]
fn 文本按_unicode_标量计数且单行拒绝换行() {
    let doc = document(vec![question(
        1,
        true,
        QuestionKind::Text { max_length: 2 },
    )]);
    // e 与组合重音是两个标量值，不是两个字节或两个字形。
    let accepted = doc
        .validate_answers(vec![answer(1, json!("e\u{301}"))])
        .unwrap();
    assert_eq!(accepted[&id(1)], json!("e\u{301}"));
    for value in ["e\u{301}x", "字\u{2028}"] {
        assert_eq!(
            doc.validate_answers(vec![answer(1, json!(value))])
                .unwrap_err()
                .question_id,
            Some(id(1))
        );
    }
}

#[test]
fn 文本题配置与答案遵守各自的字数上限() {
    for (multiline, limit) in [
        (false, MAX_TEXT_ANSWER_LENGTH),
        (true, MAX_TEXTAREA_ANSWER_LENGTH),
    ] {
        let kind = |max_length| {
            if multiline {
                QuestionKind::Textarea { max_length }
            } else {
                QuestionKind::Text { max_length }
            }
        };
        for max_length in [1, limit] {
            let doc = document(vec![question(1, true, kind(max_length))]);
            doc.validate_publish().unwrap();
            // 四字节字符仍按一个标量计数，边界原文不能被截断。
            let boundary = "𝄞".repeat(max_length);
            let accepted = doc
                .validate_answers(vec![answer(1, json!(boundary))])
                .unwrap();
            assert_eq!(accepted[&id(1)], json!(boundary));
            assert_eq!(
                doc.validate_answers(vec![answer(1, json!(format!("{boundary}字")))])
                    .unwrap_err()
                    .question_id,
                Some(id(1))
            );
        }
        let oversized = document(vec![question(1, true, kind(limit + 1))]);
        assert_eq!(
            oversized.validate_draft().unwrap_err().question_id,
            Some(id(1))
        );
        // 草稿仍可暂存未完成的零字数配置，但不能发布。
        let incomplete = document(vec![question(1, true, kind(0))]);
        incomplete.validate_draft().unwrap();
        assert_eq!(
            incomplete.validate_publish().unwrap_err().question_id,
            Some(id(1))
        );
    }
}

#[test]
fn 问卷标题说明和目的遵守各自容量() {
    let mut doc = document(vec![question(
        1,
        false,
        QuestionKind::Text { max_length: 20 },
    )]);
    doc.title = "题".repeat(200);
    doc.description = "说".repeat(MAX_TEXT_LENGTH);
    doc.purpose = "目".repeat(MAX_TEXT_LENGTH);
    doc.validate_publish().unwrap();
    doc.title.push('字');
    assert_eq!(doc.validate_draft().unwrap_err().question_id, None);
    doc.title.pop();
    doc.description.push('字');
    assert_eq!(doc.validate_draft().unwrap_err().question_id, None);
    doc.description.pop();
    doc.purpose.push('字');
    assert_eq!(doc.validate_publish().unwrap_err().question_id, None);
}

#[test]
fn 满题数问卷的原始文本和数字提交不超过发布预算() {
    let count = u128::try_from(MAX_QUESTIONS).unwrap();
    // 六字节文本转义和未规范化的 64 字节数字分别覆盖各自的预算风险。
    for (kind, value) in [
        (
            QuestionKind::Textarea {
                max_length: MAX_TEXTAREA_ANSWER_LENGTH,
            },
            json!("\u{0001}".repeat(MAX_TEXTAREA_ANSWER_LENGTH)),
        ),
        (
            QuestionKind::Number {
                integer: false,
                min: None,
                max: None,
            },
            serde_json::from_str(&format!("0.{}", "1".repeat(MAX_NUMBER_JSON_BYTES - 2))).unwrap(),
        ),
    ] {
        let doc = document(
            (1..=count)
                .map(|index| question(index, true, kind.clone()))
                .collect(),
        );
        doc.validate_publish().unwrap();
        doc.ensure_submission_budget().unwrap();
        let answers: Vec<_> = (1..=count)
            .map(|index| answer(index, value.clone()))
            .collect();
        doc.validate_answers(answers.clone()).unwrap();
        let payload = json!({
            "version_id": id(900), "idempotency_key": id(901), "answers": answers,
        });
        let bytes = serde_json::to_vec(&payload).unwrap().len();
        assert!(bytes <= doc.estimated_submission_bytes());
        assert!(bytes <= MAX_SUBMISSION_BYTES);
    }
}

#[test]
fn 数值校验拒绝空整数区间与精度丢失() {
    let mut doc = document(vec![question(
        1,
        true,
        QuestionKind::Number {
            integer: true,
            min: Some(0.1),
            max: Some(0.9),
        },
    )]);
    // 草稿允许暂时矛盾的规则，发布必须存在合法答案。
    doc.validate_draft().unwrap();
    assert_eq!(doc.validate_publish().unwrap_err().question_id, Some(id(1)));
    doc.questions[0].kind = QuestionKind::Number {
        integer: true,
        min: None,
        max: None,
    };
    // 从 JSON 原文解析，避免构造样例时先丢失精度。
    for raw in [
        "1.5",
        "9007199254740993",
        "-9223372036854775809",
        "18446744073709551617",
    ] {
        assert_eq!(
            doc.validate_answers(vec![answer(1, serde_json::from_str(raw).unwrap())])
                .unwrap_err()
                .question_id,
            Some(id(1)),
            "{raw}"
        );
    }
    for (raw, expected) in [
        ("1e20", 1e20),
        ("9007199254740992", 9_007_199_254_740_992.0),
    ] {
        let normalized = doc
            .validate_answers(vec![answer(1, serde_json::from_str(raw).unwrap())])
            .unwrap();
        assert_eq!(normalized[&id(1)].as_f64(), Some(expected));
    }
    doc.questions[0].kind = QuestionKind::Number {
        integer: false,
        min: Some(f64::NAN),
        max: None,
    };
    assert_eq!(doc.validate_draft().unwrap_err().question_id, Some(id(1)));
}

#[test]
fn 问卷和答案中的空字符在入库前拒绝() {
    let mut doc = document(vec![question(
        1,
        true,
        QuestionKind::Text { max_length: 50 },
    )]);
    doc.title = "无效\0标题".into();
    assert_eq!(doc.validate_draft().unwrap_err().question_id, None);

    doc.title = "课程反馈".into();
    let error = doc
        .validate_answers(vec![answer(1, json!("回答\0内容"))])
        .unwrap_err();
    assert_eq!(error.question_id, Some(id(1)));
}

#[test]
fn 数字边界经过_jsonb_数值表示后仍有效() {
    let doc = document(vec![question(
        1,
        true,
        QuestionKind::Number {
            integer: false,
            min: Some(-9_223_372_036_854_775_808.0),
            max: Some(0.125),
        },
    )]);
    // PostgreSQL JSONB 将 -9.223372036854776e18 输出为十进制裸整数。
    let mut stored = serde_json::to_value(doc).unwrap();
    stored["questions"][0]["min"] = serde_json::from_str("-9223372036854776000").unwrap();
    let restored: Document = serde_json::from_value(stored).unwrap();
    let accepted = restored
        .validate_answers(vec![answer(1, json!(0.125))])
        .unwrap();
    assert_eq!(accepted[&id(1)].as_f64(), Some(0.125));
    assert_eq!(
        restored
            .validate_answers(vec![answer(1, json!(0.25))])
            .unwrap_err()
            .question_id,
        Some(id(1))
    );
}

#[test]
fn 题目拼错或多余字段不能被静默丢弃() {
    let doc = document(vec![
        question(1, true, QuestionKind::SingleChoice { options: choices() }),
        question(
            2,
            false,
            QuestionKind::Number {
                integer: false,
                min: Some(5.0),
                max: Some(10.0),
            },
        ),
    ]);
    let stored = serde_json::to_value(&doc).unwrap();
    let restored: Document = serde_json::from_value(stored.clone()).unwrap();
    assert_eq!(restored, doc);

    // 把 min 拼成 minimum 时，若静默丢弃，下界会变成 null 并接受越界答案。
    let mut misspelled = stored.clone();
    let number = misspelled["questions"][1].as_object_mut().unwrap();
    let min = number.remove("min").unwrap();
    number.insert("minimum".into(), min);
    assert!(serde_json::from_value::<Document>(misspelled).is_err());

    // 公共层级的多余字段同样拒绝，不能借题型配置混入。
    let mut extra = stored.clone();
    extra["questions"][0]["shuffle"] = json!(true);
    assert!(serde_json::from_value::<Document>(extra).is_err());

    // 其他题型的合法字段也不属于本题。
    let mut foreign = stored.clone();
    foreign["questions"][1]["max_length"] = json!(20);
    assert!(serde_json::from_value::<Document>(foreign).is_err());

    let mut missing = stored;
    missing["questions"][0]
        .as_object_mut()
        .unwrap()
        .remove("required");
    assert!(serde_json::from_value::<Document>(missing).is_err());
}

#[test]
fn 混合题型预算覆盖选项和_unicode_实际编码() {
    let doc = document(vec![
        question(
            1,
            true,
            QuestionKind::Textarea {
                max_length: MAX_TEXTAREA_ANSWER_LENGTH,
            },
        ),
        question(2, true, QuestionKind::Text { max_length: 100 }),
        question(3, true, QuestionKind::SingleChoice { options: choices() }),
        question(
            4,
            true,
            QuestionKind::MultipleChoice {
                options: vec![
                    Choice {
                        id: id(103),
                        label: "选项甲".into(),
                    },
                    Choice {
                        id: id(104),
                        label: "选项乙".into(),
                    },
                ],
                min_choices: 1,
                max_choices: 2,
            },
        ),
        question(
            5,
            true,
            QuestionKind::Number {
                integer: false,
                min: None,
                max: None,
            },
        ),
    ]);
    // 覆盖默认编码的控制字符转义、四字节 UTF-8 和较长的 f64 表示。
    let answers = vec![
        answer(1, json!("\u{0001}".repeat(MAX_TEXTAREA_ANSWER_LENGTH))),
        answer(2, json!("𝄞".repeat(100))),
        answer(3, json!(id(101))),
        answer(4, json!([id(103), id(104)])),
        answer(5, json!(-1.7976931348623157e308)),
    ];
    doc.validate_answers(answers.clone()).unwrap();
    let payload = json!({
        "version_id": id(900).to_string(),
        "idempotency_key": id(901).to_string(),
        "answers": answers,
    });
    let bytes = serde_json::to_vec(&payload).unwrap().len();
    assert!(bytes <= doc.estimated_submission_bytes());
}

#[test]
fn 数字原文长度在规范化之前检查() {
    for kind in [
        QuestionKind::Number {
            integer: false,
            min: None,
            max: None,
        },
        QuestionKind::Rating {
            min: 0,
            max: 5,
            min_label: String::new(),
            max_label: String::new(),
        },
    ] {
        let doc = document(vec![question(1, true, kind)]);
        // 小数和指数均可无限补零而不改变数值；必须检查原始表示长度。
        for raw in [
            format!("1.{}", "0".repeat(MAX_NUMBER_JSON_BYTES - 2)),
            format!("1e+{}", "0".repeat(MAX_NUMBER_JSON_BYTES - 3)),
            format!("1E{}", "0".repeat(MAX_NUMBER_JSON_BYTES - 3)),
        ] {
            let accepted = doc
                .validate_answers(vec![answer(1, serde_json::from_str(&raw).unwrap())])
                .unwrap();
            assert_eq!(accepted[&id(1)].as_f64(), Some(1.0));
            let rejected = doc
                .validate_answers(vec![answer(
                    1,
                    serde_json::from_str(&format!("{raw}0")).unwrap(),
                )])
                .unwrap_err();
            assert_eq!(rejected.question_id, Some(id(1)));
        }
    }
}

#[test]
fn 流程构造与提交保留各自的文档错误优先级() {
    let mut doc = document(vec![question(
        1,
        false,
        QuestionKind::Text { max_length: 10 },
    )]);
    doc.logic = Some(DocumentLogic {
        flow: None,
        dsl_version: 2.into(),
        semantics_version: 1.into(),
        variables: Vec::new(),
        rules: Vec::new(),
    });
    doc.title = "无效\0标题".into();
    let draft_error = doc.validate_draft().unwrap_err();
    assert_eq!(doc.validate_publish().unwrap_err(), draft_error);
    assert_eq!(
        doc.validate_answers(vec![answer(99, json!(true))])
            .unwrap_err(),
        draft_error
    );
    let flow_error = FlowEvaluator::new(&doc).unwrap_err();
    assert_eq!(flow_error.code, LogicErrorCode::UnsupportedVersion);
    assert_eq!(flow_error.path, "$");
    assert_eq!(flow_error.question_id, None);

    doc.logic.as_mut().unwrap().dsl_version = 1.into();
    let flow_error = FlowEvaluator::new(&doc).unwrap_err();
    assert_eq!(flow_error.code, LogicErrorCode::InvalidFlow);
    assert_eq!(flow_error.path, "$");
    assert_eq!(flow_error.question_id, None);

    // 草稿允许未补全的题型范围；提交必须先拒绝发布错误，再检查答案身份。
    doc.title = "问卷".into();
    doc.questions[0].kind = QuestionKind::Text { max_length: 0 };
    for with_logic in [true, false] {
        if !with_logic {
            doc.logic = None;
        }
        doc.validate_draft().unwrap();
        FlowEvaluator::new(&doc).unwrap();
        let publish_error = doc.validate_publish().unwrap_err();
        assert_eq!(publish_error.question_id, Some(id(1)));
        assert_eq!(
            doc.validate_answers(vec![answer(99, json!(true))])
                .unwrap_err(),
            publish_error
        );
    }
    // 独立无规则构造仍检查题型容量，不能只保留逻辑编译检查。
    doc.questions[0].kind = QuestionKind::Text {
        max_length: MAX_TEXT_ANSWER_LENGTH + 1,
    };
    let flow_error = FlowEvaluator::new(&doc).unwrap_err();
    assert_eq!(flow_error.code, LogicErrorCode::InvalidFlow);
    assert_eq!(flow_error.question_id, Some(id(1)));
}
