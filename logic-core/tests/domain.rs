//! 领域回归守住问卷身份、草稿/发布校验、答案规范化与复制重写契约。

use std::collections::BTreeSet;

use questionnaire_logic::domain::{
    Answer, Choice, Document, DocumentLogic, FlowEvaluator, MAX_CHOICES, MAX_QUESTIONS,
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

fn node_id(node: &questionnaire_logic::domain::FlowNode) -> Uuid {
    match node {
        questionnaire_logic::domain::FlowNode::Page { id, .. }
        | questionnaire_logic::domain::FlowNode::Branch { id, .. }
        | questionnaire_logic::domain::FlowNode::End { id, .. } => *id,
    }
}

#[test]
fn 共享空白样例只通过答案校验验证必填与省略语义() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../fixtures/survey-logic/input-contracts.json"
    ))
    .unwrap();
    for case in fixtures["text_cases"].as_array().unwrap() {
        let answered = case["answered"].as_bool().unwrap();
        let value = case["value"].clone();
        for required in [false, true] {
            let doc = document(vec![question(
                1,
                required,
                QuestionKind::Textarea { max_length: 1000 },
            )]);
            let result = doc.validate_answers(vec![answer(1, value.clone())]);
            if required && !answered {
                let error = result.unwrap_err();
                assert_eq!(error.question_id, Some(id(1)), "{}", case["name"]);
                // 领域错误在映射为校验错误时保留路径，消息必须精确到可展示文案。
                assert_eq!(
                    error.message, "此题为必填项（$.answers）",
                    "{}",
                    case["name"]
                );
            } else {
                let normalized = result.unwrap();
                assert_eq!(
                    normalized.get(&id(1)),
                    answered.then_some(&value),
                    "{}",
                    case["name"]
                );
            }
        }
    }
}

#[test]
fn 草稿容量与长度上限成对生效() {
    let text_questions = |count: usize| {
        (0..count)
            .map(|index| {
                question(
                    index as u128 + 1,
                    false,
                    QuestionKind::Text { max_length: 20 },
                )
            })
            .collect::<Vec<_>>()
    };
    document(text_questions(MAX_QUESTIONS))
        .validate_draft()
        .expect("题目数量恰好等于上限应通过");
    let error = document(text_questions(MAX_QUESTIONS + 1))
        .validate_draft()
        .unwrap_err();
    assert_eq!(error.question_id, None);
    assert_eq!(
        error.message,
        format!("每份问卷最多允许 {MAX_QUESTIONS} 道题目")
    );

    let option_document = |count: usize| {
        let options = (0..count)
            .map(|index| Choice {
                id: id(1_000 + index as u128),
                label: "选项".into(),
            })
            .collect();
        document(vec![question(
            1,
            false,
            QuestionKind::SingleChoice { options },
        )])
    };
    option_document(MAX_CHOICES)
        .validate_draft()
        .expect("选项数量恰好等于上限应通过");
    let error = option_document(MAX_CHOICES + 1)
        .validate_draft()
        .unwrap_err();
    assert_eq!(error.question_id, Some(id(1)));
    assert_eq!(
        error.message,
        format!("每道选择题最多允许 {MAX_CHOICES} 个选项")
    );

    let title_document = |length: usize| {
        let mut item = question(1, false, QuestionKind::Text { max_length: 20 });
        item.title = "题".repeat(length);
        document(vec![item])
    };
    title_document(500)
        .validate_draft()
        .expect("题目标题恰好等于上限应通过");
    let error = title_document(501).validate_draft().unwrap_err();
    assert_eq!(error.question_id, Some(id(1)));
    assert_eq!(error.message, "题目标题最多允许 500 个 Unicode 字符");

    let label_document = |length: usize| {
        document(vec![question(
            1,
            false,
            QuestionKind::SingleChoice {
                options: vec![
                    Choice {
                        id: id(101),
                        label: "甲".into(),
                    },
                    Choice {
                        id: id(102),
                        label: "乙".repeat(length),
                    },
                ],
            },
        )])
    };
    label_document(500)
        .validate_draft()
        .expect("选项文字恰好等于上限应通过");
    let error = label_document(501).validate_draft().unwrap_err();
    assert_eq!(error.question_id, Some(id(1)));
    assert_eq!(error.message, "选项文字最多允许 500 个 Unicode 字符");

    let mut closing = document(vec![question(
        1,
        false,
        QuestionKind::Text { max_length: 20 },
    )]);
    closing.thank_you = "感".repeat(1_000);
    closing.validate_draft().expect("结束语恰好等于上限应通过");
    closing.thank_you.push('字');
    let error = closing.validate_draft().unwrap_err();
    assert_eq!(error.question_id, None);
    assert_eq!(error.message, "结束语最多允许 1000 个 Unicode 字符");

    let rating_document = |min_length: usize, max_length: usize| {
        document(vec![question(
            1,
            false,
            QuestionKind::Rating {
                min: 0,
                max: 5,
                min_label: "低".repeat(min_length),
                max_label: "高".repeat(max_length),
            },
        )])
    };
    rating_document(100, 100)
        .validate_draft()
        .expect("评分端点说明恰好等于上限应通过");
    let error = rating_document(101, 100).validate_draft().unwrap_err();
    assert_eq!(error.question_id, Some(id(1)));
    assert_eq!(error.message, "最低分说明最多允许 100 个 Unicode 字符");
    let error = rating_document(100, 101).validate_draft().unwrap_err();
    assert_eq!(error.question_id, Some(id(1)));
    assert_eq!(error.message, "最高分说明最多允许 100 个 Unicode 字符");
}

#[test]
fn 答案校验与流程求值共享同一委托结果() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/flows.json")).unwrap();
    let doc: Document = serde_json::from_value(fixtures["documents"]["diamond"].clone()).unwrap();
    let evaluator = FlowEvaluator::new(&doc).unwrap();
    let cases = [
        ("有效分支答案", vec![answer(1, json!(id(101).to_string()))]),
        ("缺少静态必填", Vec::new()),
        ("身份不可从其他版本混入", vec![answer(999, Value::Null)]),
        (
            "重复题目必须拒绝",
            vec![
                answer(1, json!(id(101).to_string())),
                answer(1, json!(id(102).to_string())),
            ],
        ),
    ];
    for (name, answers) in cases {
        let plain = doc.validate_answers(answers.clone());
        let flow = evaluator.evaluate_answers(answers);
        match (plain, flow) {
            (Ok(normalized), Ok(evaluated)) => {
                assert_eq!(normalized, evaluated.answers, "{name}");
            }
            (Err(plain), Err(flow)) => {
                assert_eq!(plain.question_id, flow.question_id, "{name}");
                assert_eq!(plain.message, flow.to_string(), "{name}");
            }
            (plain, flow) => panic!("两条入口必须同时成功或失败：{name}：{plain:?} / {flow:?}"),
        }
    }
}

#[test]
fn 规则编译失败映射为可定位的校验错误() {
    let mut doc = document(vec![question(
        1,
        true,
        QuestionKind::Text { max_length: 20 },
    )]);
    doc.logic = Some(
        serde_json::from_value(json!({
            "dsl_version": 1,
            "semantics_version": 1,
            "variables": [],
            "rules": [{
                "question_id": id(1),
                "visible_if": { "op": "answer", "question_id": id(999).to_string() },
            }],
            "flow": null,
        }))
        .expect("规则包结构必须合法"),
    );

    let failure = doc.compile_logic().unwrap_err();
    assert_eq!(failure.question_id, Some(id(999)));
    assert!(
        failure.path.starts_with("$.logic.rules[0].visible_if"),
        "规则错误应保留路径：{}",
        failure.path
    );

    for mapped in [
        doc.validate_draft().unwrap_err(),
        doc.validate_answers(vec![answer(1, json!("文本"))])
            .unwrap_err(),
    ] {
        assert_eq!(mapped.question_id, failure.question_id);
        assert_eq!(mapped.message, failure.to_string());
    }

    doc.logic = Some(
        serde_json::from_value::<DocumentLogic>(json!({
            "dsl_version": 2,
            "semantics_version": 1,
            "variables": [],
            "rules": [],
            "flow": null,
        }))
        .expect("版本字段仍是合法数字"),
    );
    let failure = doc.compile_logic().unwrap_err();
    assert_eq!(
        failure.code,
        questionnaire_logic::domain::LogicErrorCode::UnsupportedVersion
    );
    let mapped = doc.validate_answers(Vec::new()).unwrap_err();
    assert_eq!(mapped.question_id, None);
    assert!(mapped.message.contains("不支持此规则版本"));
}

#[test]
fn 发布态校验逐项拦截未完成内容() {
    let base = || {
        document(vec![question(
            1,
            true,
            QuestionKind::Text { max_length: 20 },
        )])
    };
    base().validate_publish().expect("基础问卷应可发布");

    let mut blank_title = base();
    blank_title.title = " \u{3000}".into();
    let mut blank_purpose = base();
    blank_purpose.purpose = String::new();
    let empty_questions = document(Vec::new());
    let mut blank_question = base();
    blank_question.questions[0].title = "\t".into();
    let single_option = document(vec![question(
        1,
        true,
        QuestionKind::SingleChoice {
            options: vec![choices()[0].clone()],
        },
    )]);
    let mut blank_option = base();
    blank_option.questions = vec![question(
        1,
        true,
        QuestionKind::MultipleChoice {
            options: vec![
                Choice {
                    id: id(101),
                    label: "选项甲".into(),
                },
                Choice {
                    id: id(102),
                    label: " ".into(),
                },
            ],
            min_choices: 1,
            max_choices: 2,
        },
    )];
    let multi = |min_choices, max_choices| {
        document(vec![question(
            1,
            true,
            QuestionKind::MultipleChoice {
                options: choices(),
                min_choices,
                max_choices,
            },
        )])
    };
    let zero_max = multi(0, 0);
    let reversed_choices = multi(3, 2);
    let too_many_choices = multi(1, 3);
    let zero_text = document(vec![question(
        1,
        true,
        QuestionKind::Text { max_length: 0 },
    )]);
    let reversed_number = document(vec![question(
        1,
        true,
        QuestionKind::Number {
            integer: false,
            min: Some(2.0),
            max: Some(1.0),
        },
    )]);
    let empty_integer_interval = document(vec![question(
        1,
        true,
        QuestionKind::Number {
            integer: true,
            min: Some(1.2),
            max: Some(1.8),
        },
    )]);
    let reversed_rating = document(vec![question(
        1,
        true,
        QuestionKind::Rating {
            min: 5,
            max: 4,
            min_label: String::new(),
            max_label: String::new(),
        },
    )]);

    let cases = [
        ("空白标题", blank_title, None, "发布前请填写问卷标题"),
        (
            "空白调查目的",
            blank_purpose,
            None,
            "发布前请说明数据收集目的",
        ),
        (
            "空题目列表",
            empty_questions,
            None,
            "发布前至少需要一道题目",
        ),
        (
            "空白题目标题",
            blank_question,
            Some(id(1)),
            "发布前请填写题目标题",
        ),
        (
            "单选题选项不足",
            single_option,
            Some(id(1)),
            "选择题至少需要两个选项",
        ),
        (
            "空白选项文字",
            blank_option,
            Some(id(1)),
            "发布前请填写所有选项文字",
        ),
        (
            "多选最大值为零",
            zero_max,
            Some(id(1)),
            "选择数量上下界必须有序，且最大值介于 1 和选项总数之间",
        ),
        (
            "多选下界大于上界",
            reversed_choices,
            Some(id(1)),
            "选择数量上下界必须有序，且最大值介于 1 和选项总数之间",
        ),
        (
            "多选上界超过选项数",
            too_many_choices,
            Some(id(1)),
            "选择数量上下界必须有序，且最大值介于 1 和选项总数之间",
        ),
        (
            "文本最大长度为零",
            zero_text,
            Some(id(1)),
            "文本最大长度至少为 1",
        ),
        (
            "数字下界大于上界",
            reversed_number,
            Some(id(1)),
            "数值上下界矛盾或区间内没有合法整数",
        ),
        (
            "整数数字题区间内无整数",
            empty_integer_interval,
            Some(id(1)),
            "数值上下界矛盾或区间内没有合法整数",
        ),
        (
            "评分上下界颠倒",
            reversed_rating,
            Some(id(1)),
            "最高分不能小于最低分",
        ),
    ];

    for (name, doc, expected_question, expected_message) in cases {
        let error = doc.validate_publish().unwrap_err();
        assert_eq!(error.question_id, expected_question, "{name}");
        assert_eq!(error.message, expected_message, "{name}");
    }

    // 筛选结束没有持久化语义，即使其余内容完整也不能发布。
    let flows: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/flows.json")).unwrap();
    let screened: Document =
        serde_json::from_value(flows["documents"]["priority"].clone()).unwrap();
    screened.validate_draft().unwrap();
    let error = screened.validate_publish().unwrap_err();
    assert_eq!(error.question_id, None);
    assert_eq!(
        error.message,
        "筛选结束的保存与限次策略尚未开放，请使用普通完成节点后再发布"
    );
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
            answer(1, json!([id(101), id(102)])),
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
        .validate_answers(vec![answer(1, json!([id(101), id(101)]))])
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
fn 数值校验拒绝空整数区间与非整数答案() {
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
    assert_eq!(
        doc.validate_answers(vec![answer(1, json!(1.5))])
            .unwrap_err()
            .question_id,
        Some(id(1))
    );
    for (raw, expected) in [("1e20", 1e20), ("42", 42.0)] {
        let normalized = doc
            .validate_answers(vec![answer(1, serde_json::from_str(raw).unwrap())])
            .unwrap();
        assert_eq!(normalized[&id(1)].as_f64(), Some(expected));
    }
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
    let error = serde_json::from_value::<Document>(misspelled).unwrap_err();
    assert!(
        error.to_string().contains("unknown field `minimum`"),
        "拼错字段必须被拒绝：{error}"
    );

    // 公共层级的多余字段同样拒绝，不能借题型配置混入。
    let mut extra = stored.clone();
    extra["questions"][0]["shuffle"] = json!(true);
    let error = serde_json::from_value::<Document>(extra).unwrap_err();
    assert!(
        error.to_string().contains("unknown field `shuffle`"),
        "题目公共字段必须严格：{error}"
    );

    // 其他题型的合法字段也不属于本题。
    let mut foreign = stored.clone();
    foreign["questions"][1]["max_length"] = json!(20);
    let error = serde_json::from_value::<Document>(foreign).unwrap_err();
    assert!(
        error.to_string().contains("unknown field `max_length`"),
        "题型配置不能混用：{error}"
    );

    let mut missing = stored;
    missing["questions"][0]
        .as_object_mut()
        .unwrap()
        .remove("required");
    let error = serde_json::from_value::<Document>(missing).unwrap_err();
    assert!(
        error.to_string().contains("missing field `required`"),
        "缺失公共字段必须被拒绝：{error}"
    );
}

#[test]
fn 普通文档复制后题目与选项身份全部重写() {
    let mut doc = document(vec![
        question(1, true, QuestionKind::Text { max_length: 20 }),
        question(2, false, QuestionKind::SingleChoice { options: choices() }),
    ]);
    let old_question_ids: Vec<Uuid> = doc.questions.iter().map(|item| item.id).collect();
    let old_option_ids: Vec<Uuid> = choices().into_iter().map(|option| option.id).collect();

    doc.renew_ids();

    let new_question_ids: Vec<Uuid> = doc.questions.iter().map(|item| item.id).collect();
    for (old, new) in old_question_ids.iter().zip(&new_question_ids) {
        assert_ne!(old, new, "题目身份必须重写");
        assert!(!new.is_nil(), "新身份不能是 Nil");
    }
    assert_eq!(
        new_question_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len(),
        new_question_ids.len(),
        "题目身份在文档内必须唯一"
    );

    let QuestionKind::SingleChoice { options } = &doc.questions[1].kind else {
        panic!("题型在复制后不能改变");
    };
    for (old, option) in old_option_ids.iter().zip(options) {
        assert_ne!(*old, option.id, "选项身份必须重写");
        assert!(!option.id.is_nil());
    }
    assert_eq!(options[0].label, "选项甲");
    assert_eq!(options[1].label, "选项乙");
    assert_eq!(doc.questions[0].title, "题目");
    assert!(doc.questions[0].required);
    assert!(!doc.questions[1].required);

    let stored = serde_json::to_string(&doc).unwrap();
    for old in old_question_ids.iter().chain(&old_option_ids) {
        assert!(
            !stored.contains(&old.to_string()),
            "复制后的文档仍残留旧身份 {old}"
        );
    }
    doc.validate_publish().unwrap();
}

#[test]
fn 带规则与流程的文档复制后引用关系保持一致() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/flows.json")).unwrap();
    let mut doc: Document =
        serde_json::from_value(fixtures["documents"]["diamond"].clone()).unwrap();

    let old_question_ids: Vec<Uuid> = doc.questions.iter().map(|item| item.id).collect();
    let old_option_ids = option_ids(&doc.questions[0].kind);
    let old_variables: Vec<Uuid> = doc
        .logic
        .as_ref()
        .unwrap()
        .variables
        .iter()
        .map(|variable| variable.id)
        .collect();
    let old_nodes: Vec<Uuid> = doc
        .logic
        .as_ref()
        .unwrap()
        .flow
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .map(node_id)
        .collect();

    doc.renew_ids();
    doc.validate_publish().expect("复制后的问卷应可发布");
    let compiled = doc
        .compile_logic()
        .unwrap()
        .expect("复制不能丢失规则编译结果");

    let logic = doc.logic.as_ref().unwrap();
    let flow = logic.flow.as_ref().unwrap();
    let new_question_ids: Vec<Uuid> = doc.questions.iter().map(|item| item.id).collect();
    let new_option_ids = option_ids(&doc.questions[0].kind);
    let new_variables: Vec<Uuid> = logic.variables.iter().map(|variable| variable.id).collect();
    let new_nodes: Vec<Uuid> = flow.nodes.iter().map(node_id).collect();

    let stored = serde_json::to_string(&doc).unwrap();
    for old in old_question_ids
        .iter()
        .chain(&old_option_ids)
        .chain(&old_variables)
        .chain(&old_nodes)
    {
        assert!(
            !stored.contains(&old.to_string()),
            "复制后的规则与流程仍残留旧身份 {old}"
        );
    }

    // 流程结构保持拓扑顺序，入口、跳转与结束结果按索引对照重写。
    assert_eq!(flow.entry, new_nodes[0]);
    match &flow.nodes[0] {
        questionnaire_logic::domain::FlowNode::Page {
            id,
            question_ids,
            next,
        } => {
            assert_eq!(*id, new_nodes[0]);
            assert_eq!(question_ids, &vec![new_question_ids[0]]);
            assert_eq!(*next, new_nodes[1]);
        }
        node => panic!("入口必须是页面：{node:?}"),
    }
    match &flow.nodes[1] {
        questionnaire_logic::domain::FlowNode::Branch {
            id,
            branches,
            otherwise,
        } => {
            assert_eq!(*id, new_nodes[1]);
            assert_eq!(branches.len(), 1);
            assert_eq!(branches[0].when["question_id"], json!(new_question_ids[0]));
            assert_eq!(branches[0].when["option_id"], json!(new_option_ids[0]));
            assert_eq!(branches[0].target, new_nodes[2]);
            assert_eq!(*otherwise, new_nodes[3]);
        }
        node => panic!("第二个节点必须是分支：{node:?}"),
    }
    for (index, expected_question) in [(2, 1), (3, 2), (4, 3)] {
        match &flow.nodes[index] {
            questionnaire_logic::domain::FlowNode::Page {
                id,
                question_ids,
                next,
            } => {
                assert_eq!(*id, new_nodes[index]);
                assert_eq!(question_ids, &vec![new_question_ids[expected_question]]);
                assert_eq!(*next, new_nodes[if index == 4 { 5 } else { 4 }]);
            }
            node => panic!("第 {index} 个节点必须是页面：{node:?}"),
        }
    }
    match &flow.nodes[5] {
        questionnaire_logic::domain::FlowNode::End { id, outcome } => {
            assert_eq!(*id, new_nodes[5]);
            assert_eq!(*outcome, questionnaire_logic::domain::FlowOutcome::Complete);
        }
        node => panic!("最后一个节点必须是结束：{node:?}"),
    }

    // 变量、规则与计算表达式中的引用必须整体迁移到新身份。
    assert_eq!(compiled.variable_order().len(), new_variables.len());
    let mut computed_order = compiled.variable_order().to_vec();
    computed_order.sort();
    let mut declared_variables = new_variables.clone();
    declared_variables.sort();
    assert_eq!(
        computed_order, declared_variables,
        "拓扑顺序必须覆盖全部重写后的变量身份"
    );
    assert_eq!(
        logic.variables[0].expression["question_id"],
        json!(new_question_ids[1])
    );
    assert_eq!(logic.rules[0].question_id, new_question_ids[1]);
    assert_eq!(logic.rules[1].question_id, new_question_ids[3]);
    assert_eq!(
        logic.rules[1].required_if.as_ref().unwrap()["value"]["left"]["variable_id"],
        json!(new_variables[0])
    );
}

/// 读取选择题选项身份；其他题型没有选项。
fn option_ids(kind: &QuestionKind) -> Vec<Uuid> {
    match kind {
        QuestionKind::SingleChoice { options } | QuestionKind::MultipleChoice { options, .. } => {
            options.iter().map(|option| option.id).collect()
        }
        _ => Vec::new(),
    }
}
