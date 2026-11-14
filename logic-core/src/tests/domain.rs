//! 领域模型内部不变量：Unicode 字符长度、草稿数值边界与六类题型答案规范化。

use serde_json::{Number, Value, json};
use uuid::Uuid;

use crate::domain::{Choice, Document, Question, QuestionKind, ValidationError, check_length};

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn 选项(value: u128) -> Choice {
    Choice {
        id: id(value),
        label: format!("选项 {value}"),
    }
}

fn 题目(value: u128, kind: QuestionKind) -> Question {
    Question {
        id: id(value),
        title: format!("题目 {value}"),
        required: false,
        kind,
    }
}

fn 数值(answer: Result<Option<Value>, ValidationError>) -> f64 {
    answer
        .expect("答案应通过规范化")
        .expect("应有规范化答案")
        .as_f64()
        .expect("规范化答案应为 JSON 数字")
}

#[test]
fn 长度检查按_unicode_标量计数并携带题目定位() {
    assert!(
        check_length("abcde", 5, None, "文本").is_ok(),
        "精确等于上限应通过"
    );
    let 错误 = check_length("abcdef", 5, None, "文本").unwrap_err();
    assert_eq!(错误.question_id, None);
    assert_eq!(错误.message, "文本最多允许 5 个 Unicode 字符");

    // 中文与四字节 Emoji 都只占一个 Unicode 标量，不按 UTF-8 字节数计算。
    assert!(
        check_length("中", 1, None, "文本").is_ok(),
        "三字节字符仍只算一个标量"
    );
    assert!(
        check_length("😀", 1, None, "文本").is_ok(),
        "四字节字符仍只算一个标量"
    );
    let 错误 = check_length("中文", 1, Some(id(7)), "题目标题").unwrap_err();
    assert_eq!(错误.question_id, Some(id(7)));
    assert_eq!(错误.message, "题目标题最多允许 1 个 Unicode 字符");

    // U+0000 即使没有超长也必须拒绝。
    let 错误 = check_length("a\0b", 10, Some(id(8)), "调查说明").unwrap_err();
    assert_eq!(错误.question_id, Some(id(8)));
    assert_eq!(错误.message, "调查说明不能包含 U+0000 空字符");
}

#[test]
fn 草稿校验拒绝非有限数值边界() {
    for (min, max, 说明) in [
        (Some(f64::INFINITY), None, "正无穷下界"),
        (Some(f64::NEG_INFINITY), None, "负无穷下界"),
        (None, Some(f64::NAN), "NaN 上界"),
    ] {
        let document = Document {
            questions: vec![题目(
                1,
                QuestionKind::Number {
                    integer: false,
                    min,
                    max,
                },
            )],
            ..Document::default()
        };
        let 错误 = document.validate_draft_fields().unwrap_err();
        assert_eq!(错误.question_id, Some(id(1)), "{说明}");
        assert_eq!(错误.message, "数值上下界必须是有限数值", "{说明}");
    }

    let document = Document {
        questions: vec![题目(
            1,
            QuestionKind::Number {
                integer: true,
                min: Some(-1.0),
                max: Some(1.0),
            },
        )],
        ..Document::default()
    };
    assert!(
        document.validate_draft_fields().is_ok(),
        "有限闭区间应通过草稿校验"
    );
}

#[test]
fn 单选答案必须是本题选项的标准_uuid() {
    let 题 = 题目(
        1,
        QuestionKind::SingleChoice {
            options: vec![选项(101), 选项(102)],
        },
    );
    assert_eq!(题.normalize_answer(None).unwrap(), None);
    assert_eq!(题.normalize_answer(Some(&Value::Null)).unwrap(), None);
    assert_eq!(
        题.normalize_answer(Some(&json!(id(101).to_string())))
            .unwrap(),
        Some(Value::String(id(101).to_string()))
    );
    assert_eq!(
        题.normalize_answer(Some(&json!(id(102).to_string().to_uppercase())))
            .unwrap(),
        Some(Value::String(id(102).to_string())),
        "大写十六进制归一为小写身份"
    );

    for (值, 消息) in [
        (json!(101), "选项 ID 必须是合法的 UUID 字符串"),
        (json!("not-a-uuid"), "选项 ID 必须是合法的 UUID 字符串"),
        (
            json!(Uuid::nil().to_string()),
            "选项 ID 必须是合法的 UUID 字符串",
        ),
        (
            json!("00000000-0000-4000-8000-00000000010A"),
            "选项不属于此版本的当前题目",
        ),
        (json!(id(999).to_string()), "选项不属于此版本的当前题目"),
    ] {
        let 错误 = 题.normalize_answer(Some(&值)).unwrap_err();
        assert_eq!(错误.question_id, Some(id(1)));
        assert_eq!(错误.message, 消息, "{值}");
    }
}

#[test]
fn 多选答案校验数量归属重复并排序归一() {
    let 题 = 题目(
        1,
        QuestionKind::MultipleChoice {
            options: vec![选项(101), 选项(102), 选项(103)],
            min_choices: 2,
            max_choices: 2,
        },
    );
    assert_eq!(
        题.normalize_answer(Some(&json!([]))).unwrap(),
        None,
        "空数组视为未回答"
    );

    let 少于 = 题
        .normalize_answer(Some(&json!([id(101).to_string()])))
        .unwrap_err();
    assert_eq!(少于.message, "所选数量不符合此题的上下限");
    let 多于 = 题
        .normalize_answer(Some(&json!([
            id(101).to_string(),
            id(102).to_string(),
            id(103).to_string()
        ])))
        .unwrap_err();
    assert_eq!(多于.message, "所选数量不符合此题的上下限");
    let 重复 = 题
        .normalize_answer(Some(&json!([id(101).to_string(), id(101).to_string()])))
        .unwrap_err();
    assert_eq!(重复.message, "多选答案不能包含重复选项");

    assert_eq!(
        题.normalize_answer(Some(&json!([id(102).to_string(), id(101).to_string()])))
            .unwrap(),
        Some(json!([id(101).to_string(), id(102).to_string()])),
        "多选结果按选项身份排序"
    );

    let 非数组 = 题
        .normalize_answer(Some(&json!(id(101).to_string())))
        .unwrap_err();
    assert_eq!(非数组.message, "多选答案必须是选项 ID 数组");
    let 外来 = 题
        .normalize_answer(Some(&json!([id(101).to_string(), id(999).to_string()])))
        .unwrap_err();
    assert_eq!(外来.message, "选项不属于此版本的当前题目");
    let 非法身份 = 题
        .normalize_answer(Some(&json!([id(101).to_string(), "not-a-uuid"])))
        .unwrap_err();
    assert_eq!(非法身份.message, "选项 ID 必须是合法的 UUID 字符串");
}

#[test]
fn 单行文本拒绝七种换行并保留非空白原文() {
    let 题 = 题目(1, QuestionKind::Text { max_length: 5 });
    assert_eq!(题.normalize_answer(Some(&json!("   "))).unwrap(), None);
    assert_eq!(
        题.normalize_answer(Some(&json!(" 原文 "))).unwrap(),
        Some(json!(" 原文 ")),
        "非空白文本不修剪"
    );

    let 非字符串 = 题.normalize_answer(Some(&json!(1))).unwrap_err();
    assert_eq!(非字符串.message, "文本答案必须是字符串");

    for 换行 in [
        '\n', '\r', '\u{000b}', '\u{000c}', '\u{0085}', '\u{2028}', '\u{2029}',
    ] {
        let 文本 = format!("前{换行}后");
        let 错误 = 题.normalize_answer(Some(&json!(文本))).unwrap_err();
        assert_eq!(
            错误.message, "单行文本不能包含换行字符",
            "换行符 U+{:04X} 应被拦截",
            换行 as u32
        );
    }

    assert_eq!(
        题.normalize_answer(Some(&json!("abcde"))).unwrap(),
        Some(json!("abcde")),
        "精确等于最大长度应通过"
    );
    let 超长 = 题.normalize_answer(Some(&json!("abcdef"))).unwrap_err();
    assert_eq!(超长.message, "文本答案最多允许 5 个 Unicode 字符");
}

#[test]
fn 多行文本允许换行但拒绝空字符() {
    let 题 = 题目(1, QuestionKind::Textarea { max_length: 10 });
    assert_eq!(
        题.normalize_answer(Some(&json!("第一行\n第二行"))).unwrap(),
        Some(json!("第一行\n第二行"))
    );
    let 空字符 = 题.normalize_answer(Some(&json!("a\0b"))).unwrap_err();
    assert_eq!(空字符.message, "文本答案不能包含 U+0000 空字符");
    assert_eq!(
        题.normalize_answer(Some(&json!(" \n\t "))).unwrap(),
        None,
        "纯空白视为未回答"
    );
}

#[test]
fn 数字答案校验有限性整数性闭区间与负零() {
    let 整数题 = 题目(
        1,
        QuestionKind::Number {
            integer: true,
            min: None,
            max: None,
        },
    );
    assert_eq!(数值(整数题.normalize_answer(Some(&json!(2)))), 2.0);
    assert_eq!(
        数值(整数题.normalize_answer(Some(&json!(2.0)))),
        2.0,
        "整数题接受 2.0 这种数学意义上的整数"
    );
    let 非整数 = 整数题.normalize_answer(Some(&json!(2.5))).unwrap_err();
    assert_eq!(非整数.message, "此题只接受整数");
    let 非数字 = 整数题.normalize_answer(Some(&json!("2"))).unwrap_err();
    assert_eq!(非数字.message, "答案必须是有限的 JSON 数字");

    let 区间题 = 题目(
        2,
        QuestionKind::Number {
            integer: false,
            min: Some(-1.5),
            max: Some(2.5),
        },
    );
    assert_eq!(
        数值(区间题.normalize_answer(Some(&json!(-1.5)))),
        -1.5,
        "闭区间下界应通过"
    );
    assert_eq!(
        数值(区间题.normalize_answer(Some(&json!(2.5)))),
        2.5,
        "闭区间上界应通过"
    );
    let 低于 = 区间题.normalize_answer(Some(&json!(-1.6))).unwrap_err();
    assert_eq!(低于.message, "数值不在允许范围内");
    let 高于 = 区间题.normalize_answer(Some(&json!(2.6))).unwrap_err();
    assert_eq!(高于.message, "数值不在允许范围内");

    let 负零 =
        数值(区间题.normalize_answer(Some(&Value::Number(Number::from_f64(-0.0).unwrap()))));
    assert_eq!(负零.to_bits(), 0.0_f64.to_bits(), "负零归一为正零");
}

#[test]
fn 评分答案必须是区间内整数() {
    let 题 = 题目(
        1,
        QuestionKind::Rating {
            min: 1,
            max: 5,
            min_label: "很差".into(),
            max_label: "很好".into(),
        },
    );
    assert_eq!(数值(题.normalize_answer(Some(&json!(1)))), 1.0);
    assert_eq!(数值(题.normalize_answer(Some(&json!(5)))), 5.0);
    for 值 in [json!(0), json!(6), json!(2.5)] {
        let 错误 = 题.normalize_answer(Some(&值)).unwrap_err();
        assert_eq!(错误.message, "评分必须是允许范围内的整数", "{值}");
    }
    let 非数字 = 题.normalize_answer(Some(&json!("3"))).unwrap_err();
    assert_eq!(非数字.message, "答案必须是有限的 JSON 数字");
}
