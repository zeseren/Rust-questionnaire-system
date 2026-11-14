//! 文档级逻辑包内部不变量：空规则包、同层变量排序、身份重写与数量上限。

use std::collections::BTreeMap;

use serde_json::{Number, Value, json};
use uuid::Uuid;

use crate::domain::{MAX_QUESTIONS, Question, QuestionKind};
use crate::logic::{LogicErrorCode, LogicValueType};
use crate::logic_document::{
    CompiledDocumentLogic, ComputedVariable, DocumentLogic, QuestionRule, rewrite,
};

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn 版本() -> Number {
    Number::from(1_u32)
}

fn 数字(value: &str) -> Value {
    json!({"op": "number", "value": value})
}

fn 变量(value: u128, value_type: LogicValueType, expression: Value) -> ComputedVariable {
    ComputedVariable {
        id: id(value),
        value_type,
        expression,
    }
}

fn 逻辑(variables: Vec<ComputedVariable>, rules: Vec<QuestionRule>) -> DocumentLogic {
    DocumentLogic {
        flow: None,
        dsl_version: 版本(),
        semantics_version: 版本(),
        variables,
        rules,
    }
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

#[test]
fn 空规则包不产生变量规则与流程() {
    let 空 = CompiledDocumentLogic::empty();
    assert_eq!(空.variable_order(), &[] as &[Uuid]);
    assert!(空.rules().is_empty());
    assert!(空.flow.is_none());

    let 值 = 空.evaluate(&BTreeMap::new()).unwrap();
    assert!(值.variables.is_empty());
    assert!(值.rules.is_empty());
}

#[test]
fn 同层变量按身份升序() {
    let logic = 逻辑(
        vec![
            变量(303, LogicValueType::Number, 数字("3")),
            变量(301, LogicValueType::Number, 数字("1")),
            变量(302, LogicValueType::Number, 数字("2")),
        ],
        Vec::new(),
    );
    let 编译结果 = logic.compile(&[]).unwrap();
    assert_eq!(
        编译结果.variable_order(),
        [id(301), id(302), id(303)].as_slice(),
        "同层无关变量按 UUID 升序，声明顺序不参与计算"
    );
}

#[test]
fn 重写覆盖全部表达式引用路径并保留未映射身份() {
    let questions = BTreeMap::from([(id(1), id(11)), (id(2), id(12))]);
    let options = BTreeMap::from([(id(101), id(201)), (id(102), id(202))]);
    let variables = BTreeMap::from([(id(301), id(401)), (id(302), id(402))]);
    let mut expression = json!({
        "op": "all",
        "expressions": [
            {"op": "answer", "question_id": id(1).to_string()},
            {"op": "answered", "question_id": id(2).to_string()},
            {"op": "variable", "variable_id": id(301).to_string()},
            {"op": "selected", "question_id": id(1).to_string(), "option_id": id(101).to_string()},
            {"op": "contains_any", "question_id": id(1).to_string(), "option_ids": [id(101).to_string(), id(102).to_string(), id(999).to_string()]},
            {"op": "contains_all", "question_id": id(1).to_string(), "option_ids": [id(102).to_string()]},
            {"op": "not", "expression": {"op": "answer", "question_id": id(1).to_string()}},
            {"op": "any", "expressions": [{"op": "answered", "question_id": id(2).to_string()}]},
            {"op": "compare", "comparison": "eq", "left": {"op": "answer", "question_id": id(1).to_string()}, "right": {"op": "number", "value": "1"}},
            {"op": "coalesce", "value": {"op": "answer", "question_id": id(2).to_string()}, "fallback": {"op": "number", "value": "0"}},
            {"op": "length", "value": {"op": "answer", "question_id": id(1).to_string()}},
            {"op": "arithmetic", "operator": "add", "left": {"op": "answer", "question_id": id(1).to_string()}, "right": {"op": "variable", "variable_id": id(302).to_string()}},
            {"op": "if", "condition": {"op": "answered", "question_id": id(1).to_string()}, "then_branch": {"op": "answer", "question_id": id(1).to_string()}, "else_branch": {"op": "answer", "question_id": id(2).to_string()}},
            {"op": "negate", "value": {"op": "answer", "question_id": id(2).to_string()}},
            {"op": "contains", "text": {"op": "variable", "variable_id": id(301).to_string()}, "substring": {"op": "answer", "question_id": id(1).to_string()}},
            {"op": "starts_with", "text": {"op": "answer", "question_id": id(2).to_string()}, "prefix": {"op": "variable", "variable_id": id(302).to_string()}},
            {"op": "answer", "question_id": id(999).to_string()},
        ],
    });
    rewrite(&mut expression, &questions, &options, &variables);
    let items = expression["expressions"]
        .as_array()
        .expect("表达式数组应保留");

    assert_eq!(items[0]["question_id"], json!(id(11)), "answer 题目引用");
    assert_eq!(items[1]["question_id"], json!(id(12)), "answered 题目引用");
    assert_eq!(items[2]["variable_id"], json!(id(401)), "variable 变量引用");
    assert_eq!(items[3]["question_id"], json!(id(11)), "selected 题目引用");
    assert_eq!(items[3]["option_id"], json!(id(201)), "selected 选项引用");
    assert_eq!(
        items[4]["option_ids"],
        json!([id(201), id(202), id(999)]),
        "contains_any 映射已配选项并保留未映射选项"
    );
    assert_eq!(
        items[5]["option_ids"],
        json!([id(202)]),
        "contains_all 选项引用"
    );
    assert_eq!(
        items[6]["expression"]["question_id"],
        json!(id(11)),
        "not 子表达式"
    );
    assert_eq!(
        items[7]["expressions"][0]["question_id"],
        json!(id(12)),
        "any 子表达式"
    );
    assert_eq!(
        items[8]["left"]["question_id"],
        json!(id(11)),
        "compare 左侧引用"
    );
    assert_eq!(
        items[9]["value"]["question_id"],
        json!(id(12)),
        "coalesce 首值引用"
    );
    assert_eq!(
        items[10]["value"]["question_id"],
        json!(id(11)),
        "length 参数引用"
    );
    assert_eq!(
        items[11]["left"]["question_id"],
        json!(id(11)),
        "arithmetic 左侧引用"
    );
    assert_eq!(
        items[11]["right"]["variable_id"],
        json!(id(402)),
        "arithmetic 右侧变量引用"
    );
    assert_eq!(
        items[12]["condition"]["question_id"],
        json!(id(11)),
        "if 条件题目引用"
    );
    assert_eq!(
        items[12]["then_branch"]["question_id"],
        json!(id(11)),
        "if 命中分支题目引用"
    );
    assert_eq!(
        items[12]["else_branch"]["question_id"],
        json!(id(12)),
        "if 未命中分支题目引用"
    );
    assert_eq!(
        items[13]["value"]["question_id"],
        json!(id(12)),
        "negate 参数题目引用"
    );
    assert_eq!(
        items[14]["text"]["variable_id"],
        json!(id(401)),
        "contains 文本变量引用"
    );
    assert_eq!(
        items[14]["substring"]["question_id"],
        json!(id(11)),
        "contains 子串题目引用"
    );
    assert_eq!(
        items[15]["text"]["question_id"],
        json!(id(12)),
        "starts_with 文本题目引用"
    );
    assert_eq!(
        items[15]["prefix"]["variable_id"],
        json!(id(402)),
        "starts_with 前缀变量引用"
    );
    assert_eq!(
        items[16]["question_id"],
        json!(id(999)),
        "未映射题目身份保持原样"
    );
}

#[test]
fn 变量与规则数量遵守题目上限边界() {
    let 变量超限: Vec<_> = (0..=MAX_QUESTIONS)
        .map(|index| 变量(1000 + index as u128, LogicValueType::Number, 数字("1")))
        .collect();
    let 错误 = 逻辑(变量超限, Vec::new()).compile(&[]).unwrap_err();
    assert_eq!(错误.code, LogicErrorCode::LimitExceeded);
    assert_eq!(错误.path, "$.logic");
    assert_eq!(错误.question_id, None);

    let 规则超限: Vec<_> = (0..=MAX_QUESTIONS)
        .map(|index| QuestionRule {
            question_id: id(2000 + index as u128),
            visible_if: Some(json!({"op": "boolean", "value": true})),
            required_if: None,
            validate: None,
        })
        .collect();
    let 错误 = 逻辑(Vec::new(), 规则超限).compile(&[]).unwrap_err();
    assert_eq!(错误.code, LogicErrorCode::LimitExceeded);
    assert_eq!(错误.path, "$.logic");
    assert_eq!(错误.question_id, None);

    // 恰好 MAX_QUESTIONS 个变量与规则仍可编译，确认拒绝只发生在越过上限之后。
    let questions: Vec<_> = (0..MAX_QUESTIONS)
        .map(|index| 数字题(index as u128 + 1))
        .collect();
    let 变量饱和: Vec<_> = (0..MAX_QUESTIONS)
        .map(|index| 变量(1000 + index as u128, LogicValueType::Number, 数字("1")))
        .collect();
    let 规则饱和: Vec<_> = (0..MAX_QUESTIONS)
        .map(|index| QuestionRule {
            question_id: id(index as u128 + 1),
            visible_if: Some(json!({"op": "boolean", "value": true})),
            required_if: None,
            validate: None,
        })
        .collect();
    let 编译结果 = 逻辑(变量饱和, 规则饱和)
        .compile(&questions)
        .expect("恰好达到上限的变量与规则应通过编译");
    assert_eq!(编译结果.variable_order().len(), MAX_QUESTIONS);
    assert_eq!(编译结果.rules().len(), MAX_QUESTIONS);
}

#[test]
fn 变量身份必须唯一() {
    let 重复 = 逻辑(
        vec![
            变量(301, LogicValueType::Number, 数字("1")),
            变量(301, LogicValueType::Number, 数字("2")),
        ],
        Vec::new(),
    );
    let 错误 = 重复.compile(&[]).unwrap_err();
    assert_eq!(错误.code, LogicErrorCode::InvalidReference);
    assert_eq!(错误.path, "$.logic.variables");
    assert_eq!(错误.question_id, None);
}

#[test]
fn 规则必须指向已知题目且至少配置一个字段() {
    let questions = [数字题(1)];
    let 未知题目 = 逻辑(
        Vec::new(),
        vec![QuestionRule {
            question_id: id(999),
            visible_if: Some(json!({"op": "boolean", "value": true})),
            required_if: None,
            validate: None,
        }],
    );
    let 错误 = 未知题目.compile(&questions).unwrap_err();
    assert_eq!(错误.code, LogicErrorCode::InvalidReference);
    assert_eq!(错误.path, "$.logic.rules[0]");
    assert_eq!(错误.question_id, Some(id(999)));

    let 空规则 = 逻辑(
        Vec::new(),
        vec![QuestionRule {
            question_id: id(1),
            visible_if: None,
            required_if: None,
            validate: None,
        }],
    );
    let 错误 = 空规则.compile(&questions).unwrap_err();
    assert_eq!(错误.code, LogicErrorCode::InvalidSyntax);
    assert_eq!(错误.path, "$.logic.rules[0]");
    assert_eq!(错误.question_id, Some(id(1)));
}
