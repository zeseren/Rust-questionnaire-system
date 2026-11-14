//! 表达式模块内部不变量：值类型名称、路径重定位、数字常量、选择器、答案读取与存在性守护。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::{MAX_CHOICES, Question, QuestionKind};
use crate::logic::{
    Binding, CompiledExpression, LogicErrorCode, LogicValueType, compile_expression, error,
    numeric, read_answer, selection,
};

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn 数字题(value: u128) -> Question {
    Question {
        id: id(value),
        title: format!("数字题 {value}"),
        required: false,
        kind: QuestionKind::Number {
            integer: false,
            min: None,
            max: None,
        },
    }
}

fn 编译(expression: &Value, questions: &[Question]) -> CompiledExpression {
    compile_expression(
        &json!({"dsl_version": 1, "semantics_version": 1, "expression": expression}),
        questions,
    )
    .expect("测试表达式应可编译")
}

#[test]
fn 值类型名称拒绝未知名称() {
    for 未知 in ["", "bool", "Choice", "choices ", "number2"] {
        assert_eq!(LogicValueType::from_name(未知), None, "{未知}");
    }
}

#[test]
fn 错误路径重定位只替换表达式前缀() {
    let 重定位 = error(
        LogicErrorCode::InvalidSyntax,
        "$.expression.left",
        Some(id(1)),
    )
    .relocate("$.logic.rules[0].visible_if");
    assert_eq!(重定位.path, "$.logic.rules[0].visible_if.left");

    // 根路径不含可替换前缀，必须保持原样。
    let 根 = error(LogicErrorCode::UnsupportedVersion, "$", None).relocate("$.logic.flow");
    assert_eq!(根.path, "$");
}

#[test]
fn 数字常量只接受无空白_json_数字() {
    for (原文, 期望) in [
        ("0", 0.0),
        ("1", 1.0),
        ("1.5", 1.5),
        ("1e3", 1000.0),
        ("-2.5", -2.5),
    ] {
        assert_eq!(numeric(原文), Some(期望), "{原文}");
    }
    assert_eq!(
        numeric("-0").map(f64::to_bits),
        Some(0.0_f64.to_bits()),
        "负零归一为正零"
    );

    for 原文 in [
        " 1", "1 ", "+1", "01", ".5", "1.", "1e400", "NaN", "Infinity", "", "-",
    ] {
        assert_eq!(numeric(原文), None, "应拒绝 {原文:?}");
    }
}

/// 选择器用例的题目绑定：单选、多选、非选择与计算变量各一题。
fn 选择绑定() -> BTreeMap<Uuid, Binding> {
    [
        (
            id(1),
            Binding {
                value_type: LogicValueType::Choice,
                options: vec![id(101), id(102)],
                variable: false,
            },
        ),
        (
            id(2),
            Binding {
                value_type: LogicValueType::Choices,
                options: vec![id(201)],
                variable: false,
            },
        ),
        (
            id(3),
            Binding {
                value_type: LogicValueType::Text,
                options: Vec::new(),
                variable: false,
            },
        ),
        (
            id(4),
            Binding {
                value_type: LogicValueType::Number,
                options: Vec::new(),
                variable: true,
            },
        ),
    ]
    .into_iter()
    .collect()
}

#[test]
fn 选择器校验题型选项归属与重复() {
    let bindings = 选择绑定();
    let 路径 = "$.expression";

    let 空 = selection(&id(1).to_string(), Vec::new(), false, 路径, &bindings).unwrap_err();
    assert_eq!(空.code, LogicErrorCode::InvalidSyntax);
    assert_eq!(空.question_id, Some(id(1)));
    assert_eq!(空.path, 路径);

    let 超量 = selection(
        &id(2).to_string(),
        vec![id(201).to_string(); MAX_CHOICES + 1],
        true,
        路径,
        &bindings,
    )
    .unwrap_err();
    assert_eq!(超量.code, LogicErrorCode::InvalidSyntax);
    assert_eq!(超量.question_id, Some(id(2)));

    let 重复 = selection(
        &id(2).to_string(),
        vec![id(201).to_string(); 2],
        true,
        路径,
        &bindings,
    )
    .unwrap_err();
    assert_eq!(重复.code, LogicErrorCode::InvalidReference);
    assert_eq!(重复.question_id, Some(id(2)));

    let 非选择 = selection(
        &id(3).to_string(),
        vec![id(101).to_string()],
        false,
        路径,
        &bindings,
    )
    .unwrap_err();
    assert_eq!(非选择.code, LogicErrorCode::TypeMismatch);
    assert_eq!(非选择.question_id, Some(id(3)));

    let 跨题 = selection(
        &id(1).to_string(),
        vec![id(201).to_string()],
        false,
        路径,
        &bindings,
    )
    .unwrap_err();
    assert_eq!(跨题.code, LogicErrorCode::InvalidReference);
    assert_eq!(跨题.question_id, Some(id(1)));

    let 未知 = selection(
        &id(999).to_string(),
        vec![id(101).to_string()],
        false,
        路径,
        &bindings,
    )
    .unwrap_err();
    assert_eq!(未知.code, LogicErrorCode::InvalidReference);
    assert_eq!(未知.question_id, Some(id(999)));

    let 变量 = selection(
        &id(4).to_string(),
        vec![id(101).to_string()],
        false,
        路径,
        &bindings,
    )
    .unwrap_err();
    assert_eq!(变量.code, LogicErrorCode::InvalidReference);
    assert_eq!(变量.question_id, Some(id(4)));

    assert_eq!(
        selection(
            &id(2).to_string(),
            vec![id(201).to_string()],
            true,
            路径,
            &bindings
        )
        .unwrap(),
        (id(2), vec![id(201)])
    );
}

#[test]
fn 答案读取拒绝越界与超量选项() {
    let 单选 = Binding {
        value_type: LogicValueType::Choice,
        options: vec![id(101)],
        variable: false,
    };
    for 值 in [json!(id(999).to_string()), json!("not-a-uuid"), json!(101)] {
        let 错误 = read_answer(Some(&值), &单选, id(4)).unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::InvalidAnswer, "{值}");
        assert_eq!(错误.question_id, Some(id(4)));
    }

    let 多选 = Binding {
        value_type: LogicValueType::Choices,
        options: vec![id(101), id(102), id(103)],
        variable: false,
    };
    let 超量 = read_answer(
        Some(&json!(vec![id(101).to_string(); MAX_CHOICES + 1])),
        &多选,
        id(5),
    )
    .unwrap_err();
    assert_eq!(超量.code, LogicErrorCode::InvalidAnswer);
}

#[test]
fn 存在性守护决定必然读取的题目集合() {
    let questions = [数字题(1), 数字题(2)];
    let 答 = |question: u128| json!({"op": "answered", "question_id": id(question).to_string()});
    let 比较 = |question: u128| {
        json!({
            "op": "compare",
            "comparison": "eq",
            "left": {"op": "answer", "question_id": id(question).to_string()},
            "right": {"op": "number", "value": "1"},
        })
    };

    // answered 只判断有无答案，本身不读取答案内容。
    let 表达式 = 编译(&答(1), &questions);
    assert!(表达式.required_reads(&BTreeMap::new()).is_empty());

    // 常量字面量没有任何题目依赖与存在性守护。
    let 常量 = 编译(&json!({"op": "boolean", "value": true}), &questions);
    assert!(常量.required_reads(&BTreeMap::new()).is_empty());

    // all 中 answered(q1) 守护同一题的 answer 读取。
    let 守护 = 编译(
        &json!({"op": "all", "expressions": [答(1), 比较(1)]}),
        &questions,
    );
    assert!(
        守护.required_reads(&BTreeMap::new()).is_empty(),
        "已由 answered(q1) 守护的读取不再计入必然读取"
    );
    let 未守护 = 编译(
        &json!({"op": "all", "expressions": [答(1), 比较(2)]}),
        &questions,
    );
    assert_eq!(
        未守护.required_reads(&BTreeMap::new()),
        BTreeSet::from([id(2)])
    );

    // any 不能保证存在性，后续读取仍保留。
    let 表达式 = 编译(
        &json!({"op": "any", "expressions": [答(1), 比较(1)]}),
        &questions,
    );
    assert_eq!(
        表达式.required_reads(&BTreeMap::new()),
        BTreeSet::from([id(1)])
    );

    // not(answered(q)) 不提供守护，后续读取仍保留。
    let 组合 = 编译(
        &json!({"op": "all", "expressions": [json!({"op": "not", "expression": 答(1)}), 比较(1)]}),
        &questions,
    );
    assert_eq!(
        组合.required_reads(&BTreeMap::new()),
        BTreeSet::from([id(1)]),
        "否定存在性不提供守护"
    );

    // coalesce 只把回退分支视为必然读取。
    let 短路 = 编译(
        &json!({"op": "coalesce", "value": {"op": "answer", "question_id": id(1).to_string()}, "fallback": {"op": "number", "value": "0"}}),
        &questions,
    );
    assert!(短路.required_reads(&BTreeMap::new()).is_empty());
    let 回退 = 编译(
        &json!({"op": "coalesce", "value": {"op": "number", "value": "0"}, "fallback": {"op": "answer", "question_id": id(2).to_string()}}),
        &questions,
    );
    assert_eq!(
        回退.required_reads(&BTreeMap::new()),
        BTreeSet::from([id(2)])
    );
}
