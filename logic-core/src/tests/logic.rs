//! 表达式模块内部不变量：值类型名称、路径重定位、数字常量、选择器、答案读取与存在性守护。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::{MAX_CHOICES, Question, QuestionKind};
use crate::logic::{
    Binding, CompiledExpression, LogicErrorCode, LogicValue, LogicValueType, compile_expression,
    error, numeric, read_answer, selection,
};
use crate::tests::support::{id, 数字题, 选项};

fn 编译(expression: &Value, questions: &[Question]) -> CompiledExpression {
    compile_expression(
        &json!({"dsl_version": 1, "semantics_version": 1, "expression": expression}),
        questions,
    )
    .expect("测试表达式应可编译")
}

fn 编译失败(expression: &Value, questions: &[Question]) -> crate::logic::LogicError {
    compile_expression(
        &json!({"dsl_version": 1, "semantics_version": 1, "expression": expression}),
        questions,
    )
    .expect_err("测试表达式应编译失败")
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
    let questions = [数字题(1, false, None, None), 数字题(2, false, None, None)];
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

fn 单选题(value: u128, options: &[u128]) -> Question {
    Question {
        id: id(value),
        title: format!("单选题 {value}"),
        required: false,
        kind: QuestionKind::SingleChoice {
            options: options.iter().map(|option| 选项(*option)).collect(),
        },
    }
}

fn 文本题(value: u128) -> Question {
    Question {
        id: id(value),
        title: format!("文本题 {value}"),
        required: false,
        kind: QuestionKind::Text { max_length: 200 },
    }
}

#[test]
fn 选项字面量绑定所属题目并拒绝跨题比较() {
    let questions = [单选题(1, &[101, 102]), 单选题(2, &[201, 202])];
    let 选项101 = json!({"op": "option", "option_id": id(101).to_string()});

    let compiled = 编译(&选项101, &questions);
    assert_eq!(compiled.value_type(), LogicValueType::Choice);
    assert_eq!(
        compiled.evaluate(&BTreeMap::new()).unwrap(),
        LogicValue::Choice(id(101)),
        "选项字面量不依赖答案即可求值"
    );
    assert!(
        compiled.required_reads(&BTreeMap::new()).is_empty(),
        "字面量不读取任何答案"
    );

    // 同题答案与选项字面量可以比较；缺失答案保持未知。
    let 同题 = 编译(
        &json!({
            "op": "compare",
            "comparison": "eq",
            "left": {"op": "answer", "question_id": id(1).to_string()},
            "right": 选项101,
        }),
        &questions,
    );
    let mut answers = BTreeMap::new();
    answers.insert(id(1), json!(id(101).to_string()));
    assert_eq!(同题.evaluate(&answers).unwrap(), LogicValue::Boolean(true));
    assert_eq!(
        同题.evaluate(&BTreeMap::new()).unwrap(),
        LogicValue::Missing
    );

    // 跨题选项域不同，编译期即拒绝。
    let 跨题 = 编译失败(
        &json!({
            "op": "compare",
            "comparison": "eq",
            "left": {"op": "answer", "question_id": id(1).to_string()},
            "right": {"op": "option", "option_id": id(201).to_string()},
        }),
        &questions,
    );
    assert_eq!(跨题.code, LogicErrorCode::TypeMismatch);
    assert_eq!(跨题.path, "$.expression");

    // 未知选项无法确定所属题目。
    let 未知 = 编译失败(
        &json!({"op": "option", "option_id": id(999).to_string()}),
        &questions,
    );
    assert_eq!(未知.code, LogicErrorCode::InvalidReference);
    assert_eq!(未知.path, "$.expression");

    // 选项域只约束单选身份：不同数字题、不同文本题之间照常比较。
    let 跨题数字与文本 = [
        文本题(3),
        文本题(5),
        数字题(4, false, None, None),
        数字题(6, false, None, None),
    ];
    for (左, 右, comparison) in [(4, 6, "ge"), (4, 6, "eq"), (3, 5, "eq")] {
        let compiled = 编译(
            &json!({
                "op": "compare",
                "comparison": comparison,
                "left": {"op": "answer", "question_id": id(左).to_string()},
                "right": {"op": "answer", "question_id": id(右).to_string()},
            }),
            &跨题数字与文本,
        );
        assert_eq!(compiled.value_type(), LogicValueType::Boolean);
    }
}

#[test]
fn 正则匹配要求整串且模式在编译期验证() {
    let questions = [文本题(3), 数字题(4, false, None, None)];
    let 文本 = || json!({"op": "answer", "question_id": id(3).to_string()});

    let compiled = 编译(
        &json!({"op": "matches", "text": 文本(), "pattern": "反.*馈"}),
        &questions,
    );
    assert_eq!(compiled.value_type(), LogicValueType::Boolean);
    assert_eq!(
        compiled.required_reads(&BTreeMap::new()),
        BTreeSet::from([id(3)])
    );
    let 答 = |text: &str| {
        let mut answers = BTreeMap::new();
        answers.insert(id(3), json!(text));
        answers
    };
    assert_eq!(
        compiled.evaluate(&答("反 馈")).unwrap(),
        LogicValue::Boolean(true)
    );
    // 整串锚定：尾部多余字符不再命中。
    assert_eq!(
        compiled.evaluate(&答("反馈之后")).unwrap(),
        LogicValue::Boolean(false)
    );
    assert_eq!(
        compiled.evaluate(&BTreeMap::new()).unwrap(),
        LogicValue::Missing
    );

    // 非法正则与超长原文都在编译期拒绝。
    let 非法 = 编译失败(
        &json!({"op": "matches", "text": 文本(), "pattern": "("}),
        &questions,
    );
    assert_eq!(非法.code, LogicErrorCode::InvalidSyntax);
    assert_eq!(非法.path, "$.expression");

    let 超长 = "a".repeat(257);
    let 超限 = 编译失败(
        &json!({"op": "matches", "text": 文本(), "pattern": 超长}),
        &questions,
    );
    assert_eq!(超限.code, LogicErrorCode::LimitExceeded);

    // 非文本操作数不进入匹配。
    let 类型 = 编译失败(
        &json!({"op": "matches", "text": {"op": "number", "value": "1"}, "pattern": "1"}),
        &questions,
    );
    assert_eq!(类型.code, LogicErrorCode::TypeMismatch);

    // 原文不平衡时不能被锚定包裹“修复”成可编译且失去整串语义的模式。
    let 不平衡 = 编译失败(
        &json!({"op": "matches", "text": 文本(), "pattern": "a)|(b"}),
        &questions,
    );
    assert_eq!(不平衡.code, LogicErrorCode::InvalidSyntax);
}

#[test]
fn 舍入算子按十进制缩放并校验位数() {
    let questions = [数字题(4, false, None, None)];
    let 数字 = |raw: &str| json!({"op": "number", "value": raw});
    let 求值 = |expression: Value| {
        编译(&expression, &questions)
            .evaluate(&BTreeMap::new())
            .unwrap()
    };

    // 0.1 + 0.2 在 2 位小数下舍入回 0.3。
    assert_eq!(
        求值(json!({
            "op": "round",
            "value": {
                "op": "arithmetic",
                "operator": "add",
                "left": 数字("0.1"),
                "right": 数字("0.2"),
            },
            "digits": 2,
        })),
        LogicValue::Number(0.3)
    );
    // 负数四舍五入远离零；floor/ceil 分别向两侧取整。
    assert_eq!(
        求值(json!({"op": "round", "value": 数字("-2.5"), "digits": 0})),
        LogicValue::Number(-3.0)
    );
    assert_eq!(
        求值(json!({"op": "floor", "value": 数字("-1.5"), "digits": 0})),
        LogicValue::Number(-2.0)
    );
    assert_eq!(
        求值(json!({"op": "ceil", "value": 数字("-1.5"), "digits": 0})),
        LogicValue::Number(-1.0)
    );
    assert_eq!(
        求值(json!({"op": "floor", "value": 数字("1.239"), "digits": 2})),
        LogicValue::Number(1.23)
    );
    assert_eq!(
        求值(json!({"op": "ceil", "value": 数字("1.231"), "digits": 2})),
        LogicValue::Number(1.24)
    );
    // 已落在目标精度上的值缩放后带二进制误差，floor/ceil 不能再跳一格。
    for (op, raw, expected) in [
        ("floor", "0.29", 0.29),
        ("floor", "4.35", 4.35),
        ("floor", "0.57", 0.57),
        ("ceil", "0.07", 0.07),
        ("ceil", "0.14", 0.14),
    ] {
        assert_eq!(
            求值(json!({"op": op, "value": 数字(raw), "digits": 2})),
            LogicValue::Number(expected),
            "{op}({raw}, 2)"
        );
    }

    // 舍入读取其数字子表达式，缺失操作数保持未知。
    let 缺失 = 编译(
        &json!({"op": "round", "value": {"op": "answer", "question_id": id(4).to_string()}, "digits": 2}),
        &questions,
    );
    assert_eq!(
        缺失.required_reads(&BTreeMap::new()),
        BTreeSet::from([id(4)])
    );
    assert_eq!(
        缺失.evaluate(&BTreeMap::new()).unwrap(),
        LogicValue::Missing
    );

    // 越界、非整数或缺失位数均拒绝。
    for digits in [json!(7), json!(1.5), json!(-1)] {
        let 错误 = 编译失败(
            &json!({"op": "round", "value": 数字("1"), "digits": digits}),
            &questions,
        );
        assert_eq!(错误.code, LogicErrorCode::InvalidSyntax, "{digits}");
    }
    let 缺位数 = compile_expression(
        &json!({
            "dsl_version": 1,
            "semantics_version": 1,
            "expression": {"op": "round", "value": 数字("1")},
        }),
        &questions,
    )
    .unwrap_err();
    assert_eq!(缺位数.code, LogicErrorCode::InvalidSyntax);

    // 缩放后溢出按算术错误拒绝。
    let 溢出 = 编译(
        &json!({"op": "round", "value": 数字("1e308"), "digits": 2}),
        &questions,
    );
    assert_eq!(
        溢出.evaluate(&BTreeMap::new()).unwrap_err().code,
        LogicErrorCode::ArithmeticError
    );
}
