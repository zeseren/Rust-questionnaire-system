//! 领域回归验证保持数字精度、历史答案兼容性与完整提交的容量保证。
use questionnaire_api::domain::{
    Answer, Choice, Document, MAX_NUMBER_JSON_BYTES, MAX_QUESTIONS, MAX_SUBMISSION_BYTES,
    MAX_TEXT_ANSWER_LENGTH, MAX_TEXT_LENGTH, MAX_TEXTAREA_ANSWER_LENGTH, Question, QuestionKind,
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
            max_length: MAX_TEXT_ANSWER_LENGTH,
        },
    )]);
    let boundary = "汉".repeat(MAX_TEXT_ANSWER_LENGTH);
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
            doc.validate_draft().unwrap();
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
        assert_eq!(
            oversized.validate_publish().unwrap_err().question_id,
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
fn 复制草稿沿用历史文本容量而保存发布仍按当前上限() {
    // 容量收紧前发布的历史问卷允许 5000 字文本题；复制必须仍能生成可编辑草稿，
    // 但草稿自身、保存和发布继续按当前上限定位超限题目。
    for multiline in [false, true] {
        let legacy_kind = |max_length| {
            if multiline {
                QuestionKind::Textarea { max_length }
            } else {
                QuestionKind::Text { max_length }
            }
        };
        let historical = document(vec![question(1, true, legacy_kind(MAX_TEXT_LENGTH))]);
        assert_eq!(
            historical.validate_draft().unwrap_err().question_id,
            Some(id(1))
        );
        assert_eq!(
            historical.validate_publish().unwrap_err().question_id,
            Some(id(1))
        );
        historical.validate_copy_draft().unwrap();

        // 复制不是绕过结构校验的入口：超过历史容量的配置仍然拒绝。
        let mut oversized = historical.clone();
        oversized.questions[0].kind = legacy_kind(MAX_TEXT_LENGTH + 1);
        assert_eq!(
            oversized.validate_copy_draft().unwrap_err().question_id,
            Some(id(1))
        );
    }
}

#[test]
fn 答案上限收紧不改变调查说明和收集目的的容量() {
    let mut doc = document(vec![question(
        1,
        false,
        QuestionKind::Text { max_length: 20 },
    )]);
    doc.description = "说".repeat(MAX_TEXT_LENGTH);
    doc.purpose = "目".repeat(MAX_TEXT_LENGTH);
    doc.validate_draft().unwrap();
    doc.validate_publish().unwrap();
    doc.description.push('字');
    assert_eq!(doc.validate_draft().unwrap_err().question_id, None);
    doc.description.pop();
    doc.purpose.push('字');
    assert_eq!(doc.validate_publish().unwrap_err().question_id, None);
}

#[test]
fn 历史文本题沿用已发布的字数限制且仍校验题面() {
    for kind in [
        QuestionKind::Text {
            max_length: MAX_TEXT_LENGTH,
        },
        QuestionKind::Textarea {
            max_length: MAX_TEXT_LENGTH,
        },
    ] {
        let mut doc = document(vec![question(1, true, kind)]);
        assert_eq!(doc.validate_draft().unwrap_err().question_id, Some(id(1)));
        assert_eq!(doc.validate_publish().unwrap_err().question_id, Some(id(1)));
        let boundary = "汉".repeat(MAX_TEXT_LENGTH);
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
        // 放宽历史容量不会绕过必填、身份唯一或题面完整性校验。
        assert_eq!(
            doc.validate_answers(Vec::new()).unwrap_err().question_id,
            Some(id(1))
        );
        doc.questions.push(doc.questions[0].clone());
        assert_eq!(
            doc.validate_answers(Vec::new()).unwrap_err().question_id,
            Some(id(1))
        );
        doc.questions.pop();
        doc.title.clear();
        assert_eq!(
            doc.validate_answers(vec![answer(1, json!("回答"))])
                .unwrap_err()
                .question_id,
            None
        );
    }
}

#[test]
fn 一百道多行文本题全部写满仍可发布并提交() {
    let count = u128::try_from(MAX_QUESTIONS).unwrap();
    let doc = document(
        (1..=count)
            .map(|index| {
                question(
                    index,
                    true,
                    QuestionKind::Textarea {
                        max_length: MAX_TEXTAREA_ANSWER_LENGTH,
                    },
                )
            })
            .collect(),
    );
    doc.validate_publish().unwrap();
    doc.ensure_submission_budget().unwrap();
    assert!(doc.estimated_submission_bytes() <= MAX_SUBMISSION_BYTES);
    // 不只测常见汉字；同时覆盖六字节转义和四字节 UTF-8 的实际载荷。
    for scalar in ["汉", "\u{0001}", "𝄞"] {
        let answers: Vec<_> = (1..=count)
            .map(|index| answer(index, json!(scalar.repeat(MAX_TEXTAREA_ANSWER_LENGTH))))
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
    // 历史题面可含 5000 字文本题，预算保护仍能拒绝超过请求上限的配置。
    assert!(oversized.estimated_submission_bytes() > MAX_SUBMISSION_BYTES);
    assert_eq!(
        oversized
            .ensure_submission_budget()
            .unwrap_err()
            .question_id,
        None
    );
    // 新发布拒绝旧容量配置，但历史版本的小载荷答卷仍可校验。
    assert_eq!(
        oversized.validate_publish().unwrap_err().question_id,
        Some(id(1))
    );
    assert!(
        oversized
            .validate_answers((1..=60).map(|index| answer(index, json!("回答"))).collect())
            .is_ok()
    );

    let within = document(
        (1..=34)
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
fn text_budget_covers_json_escaping_and_quotes_at_request_boundary() {
    let mut doc = document(
        (1..=100)
            .map(|index| {
                question(
                    index,
                    true,
                    QuestionKind::Text {
                        max_length: if index <= 20 { 1737 } else { 1736 },
                    },
                )
            })
            .collect(),
    );
    // 历史题面保留请求边界回归：100 条答案使引号的遗漏无法被信封余量掩盖。
    assert_eq!(doc.estimated_submission_bytes(), MAX_SUBMISSION_BYTES);
    doc.ensure_submission_budget().unwrap();
    for scalar in ["\u{0001}", "𝄞", "\"", "\\"] {
        let answers: Vec<_> = (1..=100)
            .map(|index| {
                answer(
                    index,
                    json!(scalar.repeat(if index <= 20 { 1737 } else { 1736 })),
                )
            })
            .collect();
        doc.validate_answers(answers.clone()).unwrap();
        let payload = json!({
            "version_id": id(900), "idempotency_key": id(901), "answers": answers,
        });
        assert!(serde_json::to_vec(&payload).unwrap().len() <= doc.estimated_submission_bytes());
    }
    doc.questions[0].kind = QuestionKind::Text { max_length: 1738 };
    assert_eq!(
        doc.ensure_submission_budget().unwrap_err().question_id,
        None
    );
}

#[test]
fn numeric_json_length_is_checked_before_normalization() {
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
fn submission_budget_covers_unrounded_numeric_payloads() {
    // 全数字问卷单独验证数字预算，混合问卷复现审查发现的超过 1 MiB 载荷。
    for text_count in [0, 52] {
        let doc = document(
            (1..=100)
                .map(|index| {
                    question(
                        index,
                        true,
                        if index <= text_count {
                            QuestionKind::Textarea {
                                max_length: MAX_TEXT_LENGTH,
                            }
                        } else {
                            QuestionKind::Number {
                                integer: false,
                                min: None,
                                max: None,
                            }
                        },
                    )
                })
                .collect(),
        );
        let raw: Value =
            serde_json::from_str(&format!("0.{}", "1".repeat(MAX_NUMBER_JSON_BYTES - 2))).unwrap();
        let answers: Vec<_> = (1..=100)
            .map(|index| {
                answer(
                    index,
                    if index <= text_count {
                        json!("𝄞".repeat(MAX_TEXT_LENGTH))
                    } else {
                        raw.clone()
                    },
                )
            })
            .collect();
        doc.validate_answers(answers.clone()).unwrap();
        let payload = json!({
            "version_id": id(900), "idempotency_key": id(901), "answers": answers,
        });
        let bytes = serde_json::to_vec(&payload).unwrap().len();
        assert!(bytes <= doc.estimated_submission_bytes());
        if text_count == 0 {
            doc.ensure_submission_budget().unwrap();
        } else {
            assert!(bytes > MAX_SUBMISSION_BYTES);
            assert_eq!(
                doc.ensure_submission_budget().unwrap_err().question_id,
                None
            );
        }
    }
}
