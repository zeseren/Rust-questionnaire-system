use questionnaire_api::domain::{
    Answer, Choice, Document, MAX_SUBMISSION_BYTES, MAX_TEXT_LENGTH, Question, QuestionKind,
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
fn normalization_ignores_answer_order_choice_order_and_number_spelling() {
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
fn duplicate_and_foreign_identities_are_rejected_instead_of_overwritten() {
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
    doc.renew_ids();
    doc.validate_publish().unwrap();
    assert!(
        !doc.questions
            .iter()
            .any(|question| question.id == id(1) || question.id == id(2))
    );
}

#[test]
fn missing_optional_answers_are_omitted_but_zero_is_a_real_required_answer() {
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
fn text_boundaries_count_unicode_scalars_and_reject_single_line_breaks() {
    let mut doc = document(vec![question(
        1,
        true,
        QuestionKind::Text {
            max_length: MAX_TEXT_LENGTH,
        },
    )]);
    let boundary = "汉".repeat(MAX_TEXT_LENGTH);
    let accepted = doc
        .validate_answers(vec![answer(1, json!(boundary))])
        .unwrap();
    assert_eq!(accepted[&id(1)], json!(boundary));
    let too_long = doc
        .validate_answers(vec![answer(1, json!(format!("{boundary}字")))])
        .unwrap_err();
    assert_eq!(too_long.question_id, Some(id(1)));
    let line_break = doc
        .validate_answers(vec![answer(1, json!("第一行\u{2028}第二行"))])
        .unwrap_err();
    assert_eq!(line_break.question_id, Some(id(1)));

    doc.questions[0].kind = QuestionKind::Text { max_length: 2 };
    // e + 组合重音是两个标量值，并不是一个字节或一个字形。
    let combining = doc
        .validate_answers(vec![answer(1, json!("e\u{301}"))])
        .unwrap();
    assert_eq!(combining[&id(1)], json!("e\u{301}"));
    assert_eq!(
        doc.validate_answers(vec![answer(1, json!("e\u{301}x"))])
            .unwrap_err()
            .question_id,
        Some(id(1))
    );
    doc.title = "题".repeat(200);
    doc.validate_draft().unwrap();
    doc.title.push('字');
    assert_eq!(doc.validate_draft().unwrap_err().question_id, None);
}

#[test]
fn numeric_validation_rejects_empty_integer_ranges_and_precision_loss() {
    let mut doc = document(vec![question(
        1,
        true,
        QuestionKind::Number {
            integer: true,
            min: Some(0.1),
            max: Some(0.9),
        },
    )]);
    // 草稿允许暂时矛盾的规则，但发布不能产生一道无合法整数答案的题。
    doc.validate_draft().unwrap();
    assert_eq!(doc.validate_publish().unwrap_err().question_id, Some(id(1)));
    doc.questions[0].kind = QuestionKind::Number {
        integer: true,
        min: None,
        max: None,
    };
    assert_eq!(
        doc.validate_answers(vec![answer(1, json!(1.5))])
            .unwrap_err()
            .question_id,
        Some(id(1))
    );
    assert_eq!(
        doc.validate_answers(vec![answer(1, json!(9_007_199_254_740_993_u64))])
            .unwrap_err()
            .question_id,
        Some(id(1))
    );
    // 必须从原始 JSON 解析，防止测试构造值时就已经丢失超界整数的精度。
    let below_i64: Value = serde_json::from_str("-9223372036854775809").unwrap();
    assert_eq!(
        doc.validate_answers(vec![answer(1, below_i64)])
            .unwrap_err()
            .question_id,
        Some(id(1))
    );
    let above_u64: Value = serde_json::from_str("18446744073709551617").unwrap();
    assert_eq!(
        doc.validate_answers(vec![answer(1, above_u64)])
            .unwrap_err()
            .question_id,
        Some(id(1))
    );
    let exponent: Value = serde_json::from_str("1e20").unwrap();
    let normalized = doc.validate_answers(vec![answer(1, exponent)]).unwrap();
    assert_eq!(normalized[&id(1)].as_f64(), Some(1e20));
    let exact = doc
        .validate_answers(vec![answer(1, json!(9_007_199_254_740_992_u64))])
        .unwrap();
    assert_eq!(exact[&id(1)].as_f64(), Some(9_007_199_254_740_992.0));
    doc.questions[0].kind = QuestionKind::Number {
        integer: false,
        min: Some(f64::NAN),
        max: None,
    };
    assert_eq!(doc.validate_draft().unwrap_err().question_id, Some(id(1)));
}

#[test]
fn nul_in_document_or_answer_is_rejected_before_persistence() {
    let mut doc = document(vec![question(
        1,
        true,
        QuestionKind::Text { max_length: 50 },
    )]);
    doc.title = "invalid\0title".into();
    assert_eq!(doc.validate_draft().unwrap_err().question_id, None);

    doc.title = "课程反馈".into();
    let error = doc
        .validate_answers(vec![answer(1, json!("回答\0内容"))])
        .unwrap_err();
    assert_eq!(error.question_id, Some(id(1)));
}

#[test]
fn numeric_bounds_survive_json_storage_round_trip() {
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
fn misspelled_question_fields_are_rejected_instead_of_dropped() {
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
fn submission_budget_rejects_surveys_that_cannot_fit_one_request() {
    let oversized = document(
        (1..=60)
            .map(|index| {
                question(
                    index,
                    true,
                    QuestionKind::Textarea {
                        max_length: MAX_TEXT_LENGTH,
                    },
                )
            })
            .collect(),
    );
    // 单题合法，但整份最坏情况超过请求上限，发布必须提前拒绝而不是让填写者 413。
    assert!(oversized.estimated_submission_bytes() > MAX_SUBMISSION_BYTES);
    assert_eq!(
        oversized
            .ensure_submission_budget()
            .unwrap_err()
            .question_id,
        None
    );
    // 题面本身仍然合法：预算与发布规则是两道独立关卡。
    assert!(oversized.validate_publish().is_ok());

    let within = document(
        (1..=40)
            .map(|index| {
                question(
                    index,
                    true,
                    QuestionKind::Textarea {
                        max_length: MAX_TEXT_LENGTH,
                    },
                )
            })
            .collect(),
    );
    assert!(within.estimated_submission_bytes() <= MAX_SUBMISSION_BYTES);
    assert!(within.ensure_submission_budget().is_ok());
}

#[test]
fn estimated_submission_bytes_covers_a_worst_case_payload() {
    let doc = document(vec![
        question(
            1,
            true,
            QuestionKind::Textarea {
                max_length: MAX_TEXT_LENGTH,
            },
        ),
        question(2, true, QuestionKind::Text { max_length: 100 }),
        question(3, true, QuestionKind::SingleChoice { options: choices() }),
        question(
            4,
            true,
            QuestionKind::MultipleChoice {
                options: choices(),
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
    // 最坏情况载荷：每个标量占满 4 字节 UTF-8，数字取 f64 最长表示。
    let answers = vec![
        answer(1, json!("𝄞".repeat(MAX_TEXT_LENGTH))),
        answer(2, json!("𝄞".repeat(100))),
        answer(3, json!(id(101))),
        answer(4, json!([id(101), id(102)])),
        answer(5, json!(-1.7976931348623157e308)),
    ];
    let payload = json!({
        "version_id": id(900).to_string(),
        "idempotency_key": id(901).to_string(),
        "answers": answers,
    });
    let bytes = serde_json::to_vec(&payload).unwrap().len();
    assert!(bytes <= doc.estimated_submission_bytes());
}
