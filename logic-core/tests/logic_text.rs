//! 文本 DSL 回归：共享人工期望、格式化往返幂等、文法负例与函数签名校验。

use questionnaire_logic::domain::{
    Document, DocumentLogic, LogicErrorCode, LogicTextError, Question, format_logic_text,
    inspect_logic_text, parse_logic_text,
};
use serde_json::{Value, json};

fn texts_fixture() -> Value {
    serde_json::from_str(include_str!("../../fixtures/survey-logic/texts.json")).unwrap()
}

fn flows_fixture() -> Value {
    serde_json::from_str(include_str!("../../fixtures/survey-logic/flows.json")).unwrap()
}

fn document() -> Document {
    let fixture = texts_fixture();
    let flows = flows_fixture();
    serde_json::from_value(flows["documents"][fixture["document"].as_str().unwrap()].clone())
        .unwrap()
}

fn parse_program(source: &str, questions: &[Question]) -> Result<DocumentLogic, LogicTextError> {
    parse_logic_text(source, questions)
}

/// 格式样例字段：显式值优先，否则按 `document` 名读取共享流程文档；无效引用必须报错。
fn format_case_field<'a>(item: &'a Value, flows: &'a Value, field: &str) -> &'a Value {
    if let Some(value) = item.get(field) {
        return value;
    }
    let name = item["name"].as_str().unwrap_or("未命名");
    let document_name = item["document"]
        .as_str()
        .unwrap_or_else(|| panic!("格式样例 {name} 缺少 {field} 或 document 引用"));
    let document = flows["documents"]
        .get(document_name)
        .unwrap_or_else(|| panic!("格式样例 {name} 引用了未知流程文档 {document_name}"));
    document
        .get(field)
        .unwrap_or_else(|| panic!("流程文档 {document_name} 缺少 {field} 字段"))
}

#[test]
fn 文本共享样例保留身份优先级并定位语法与语义错误() {
    let fixture = texts_fixture();
    let document = document();
    for item in fixture["cases"].as_array().unwrap() {
        let mut source = fixture["source_lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        for replacement in item["replacements"].as_array().unwrap() {
            source = source.replace(
                replacement[0].as_str().unwrap(),
                replacement[1].as_str().unwrap(),
            );
        }
        source.push_str(item["append"].as_str().unwrap_or(""));
        let result = parse_logic_text(&source, &document.questions);
        if item.get("error").is_some() {
            let error = result.unwrap_err();
            assert_eq!(
                json!({"code":error.error.code,"line":error.line,"column":error.column}),
                json!({"code":item["error"],"line":item["line"],"column":item["column"]}),
                "{}",
                item["name"]
            );
        } else {
            let mut expected = serde_json::to_value(document.logic.as_ref().unwrap()).unwrap();
            for edit in item["edits"].as_array().unwrap() {
                let mut target = &mut expected;
                for key in edit["path"].as_array().unwrap() {
                    target = if let Some(index) = key.as_u64() {
                        &mut target[index as usize]
                    } else {
                        &mut target[key.as_str().unwrap()]
                    };
                }
                *target = edit["value"].clone();
            }
            assert_eq!(
                serde_json::to_value(result.unwrap()).unwrap(),
                expected,
                "{}",
                item["name"]
            );
        }
    }
}

#[test]
fn 文本文法负例统一拒绝并保留行列位置() {
    let cases = [
        ("缺少头部结束符", "dsl 1 semantics 1", 1),
        ("未知结束结果", "dsl 1 semantics 1;\nend n1 = done;", 2),
        (
            "未闭合的绑定字符串",
            "dsl 1 semantics 1;\nbind question topic = \"abc;",
            2,
        ),
        ("非法词法字符", "dsl 1 semantics 1;\n@", 2),
        (
            "页面缺少跳转箭头",
            concat!(
                "dsl 1 semantics 1;\n",
                "bind node a = \"00000000-0000-4000-8000-000000001001\";\n",
                "bind node b = \"00000000-0000-4000-8000-000000001002\";\n",
                "entry a;\n",
                "page a [] b;\n",
                "end b = complete;\n",
            ),
            5,
        ),
    ];
    for (name, source, expected_line) in cases {
        let error = parse_program(source, &[]).unwrap_err();
        assert_eq!(error.error.code, LogicErrorCode::InvalidSyntax, "{name}");
        assert_eq!(error.error.path, "$", "{name}");
        assert_eq!(error.error.question_id, None, "{name}");
        assert_eq!(error.line, expected_line, "{name}");
        assert!(error.column >= 1, "{name} 必须给出列号");
    }
}

#[test]
fn 内置函数检查参数元数与引用种类() {
    let prefix = "dsl 1 semantics 1; bind variable v = \"00000000-0000-4000-8000-000000002001\"; let v: number = ";
    for expression in [
        "number(1)",
        "number()",
        "number(1, 2)",
        "coalesce(1)",
        "coalesce(1, 2, 3)",
        "answer()",
        "length([])",
        "contains(1)",
        "contains(1, 2, 3)",
        "starts_with(1)",
    ] {
        let error = parse_program(&format!("{prefix}{expression};"), &[]).unwrap_err();
        assert_eq!(
            error.error.code,
            LogicErrorCode::InvalidSyntax,
            "参数元数或种类错误必须按语法拒绝：{expression}"
        );
        assert_eq!(error.line, 1);
    }

    // 引用函数按类别解析别名，第二个参数不是数组时必须按语法拒绝。
    let document = document();
    let base = concat!(
        "dsl 1 semantics 1;\n",
        "bind question topic = \"00000000-0000-4000-8000-000000000001\";\n",
        "bind option opt = \"00000000-0000-4000-8000-000000000101\";\n",
        "bind variable calc = \"00000000-0000-4000-8000-000000002001\";\n",
    );
    let non_array = parse_error(
        &format!("{base}let calc: boolean = contains_any(topic, calc);"),
        &document.questions,
    );
    assert_eq!(non_array.error.code, LogicErrorCode::InvalidSyntax);
    assert_eq!(non_array.line, 5);
}

fn parse_error(source: &str, questions: &[Question]) -> LogicTextError {
    parse_program(source, questions).unwrap_err()
}

#[test]
fn 文本与表达式预算允许边界并拒绝长链和混合嵌套超限() {
    let prefix = "dsl 1 semantics 1; bind variable v = \"00000000-0000-4000-8000-000000002001\"; let v: boolean = ";
    for expression in [
        format!("{}true{}", "(".repeat(63), ")".repeat(63)),
        format!("!{}true{}", "(".repeat(62), ")".repeat(62)),
        format!("all({}true{})", "(".repeat(62), ")".repeat(62)),
    ] {
        assert!(parse_program(&format!("{prefix}{expression};"), &[]).is_ok());
    }
    for expression in [
        format!("{}true{}", "(".repeat(64), ")".repeat(64)),
        format!("!all({}true{})", "(".repeat(62), ")".repeat(62)),
    ] {
        let error = parse_program(&format!("{prefix}{expression};"), &[]).unwrap_err();
        assert_eq!(error.error.code, LogicErrorCode::LimitExceeded);
    }
    assert_eq!(
        parse_program(&" ".repeat(1_048_577), &[])
            .unwrap_err()
            .error
            .code,
        LogicErrorCode::LimitExceeded
    );
}

#[test]
fn 一元负号与三元条件解析为统一表达式节点() {
    let document = document();
    let prefix = concat!(
        "dsl 1 semantics 1;\n",
        "bind question amount = \"00000000-0000-4000-8000-000000000002\";\n",
        "bind variable v = \"00000000-0000-4000-8000-000000002001\";\n",
        "let v: number = ",
    );

    // 前缀负号与数字之间允许空白，并与 `!` 使用同一预算入口。
    let logic = parse_program(&format!("{prefix}- 5;"), &document.questions).unwrap();
    assert_eq!(
        logic.variables[0].expression,
        json!({"op": "negate", "value": {"op": "number", "value": "5"}})
    );

    // `-answer(...)` 解析为负号作用于答案，而不是二元减法。
    let logic = parse_program(&format!("{prefix}-answer(amount);"), &document.questions).unwrap();
    assert_eq!(
        logic.variables[0].expression,
        json!({
            "op": "negate",
            "value": {
                "op": "answer",
                "question_id": "00000000-0000-4000-8000-000000000002"
            }
        })
    );

    let logic = parse_program(
        &format!("{prefix}if answer(amount) > 0 then -1 else 1;"),
        &document.questions,
    )
    .unwrap();
    assert_eq!(
        logic.variables[0].expression,
        json!({
            "op": "if",
            "condition": {
                "op": "compare",
                "comparison": "gt",
                "left": {
                    "op": "answer",
                    "question_id": "00000000-0000-4000-8000-000000000002"
                },
                "right": {"op": "number", "value": "0"}
            },
            "then_branch": {"op": "negate", "value": {"op": "number", "value": "1"}},
            "else_branch": {"op": "number", "value": "1"}
        })
    );
}

#[test]
fn 保留关键字不能用作别名或引用名() {
    let document = document();
    let header = "dsl 1 semantics 1;\n";
    for alias in ["if", "then", "else", "true", "false"] {
        let source =
            format!("{header}bind question {alias} = \"00000000-0000-4000-8000-000000000001\";");
        assert_eq!(
            parse_program(&source, &document.questions)
                .unwrap_err()
                .error
                .code,
            LogicErrorCode::InvalidSyntax,
            "别名 {alias} 必须被文法拒绝"
        );
        let source =
            format!("{header}bind node {alias} = \"00000000-0000-4000-8000-0000000000aa\";");
        assert_eq!(
            parse_program(&source, &document.questions)
                .unwrap_err()
                .error
                .code,
            LogicErrorCode::InvalidSyntax,
            "节点别名 {alias} 必须被文法拒绝"
        );
    }
    // 前缀相近但并非保留字的别名仍然合法。
    let source = format!("{header}bind question iffy = \"00000000-0000-4000-8000-000000000001\";");
    assert!(
        parse_program(&source, &document.questions).is_ok(),
        "非保留字前缀必须继续可用"
    );
}

#[test]
fn 巡检接口在语法错误后仍返回已扫描绑定() {
    let document = document();
    let source = concat!(
        "dsl 1 semantics 1;\n",
        "bind question choose = \"00000000-0000-4000-8000-000000000001\";\n",
        "bind option yes = \"00000000-0000-4000-8000-000000000101\";\n",
        "let broken = ;",
    );
    let inspection = inspect_logic_text(source, &document.questions);
    assert_eq!(inspection.bindings.len(), 2, "报错不能清空速查栏");
    assert_eq!(inspection.bindings[0].category, "question");
    assert_eq!(inspection.bindings[0].alias, "choose");
    assert_eq!(inspection.bindings[1].category, "option");
    let error = inspection.error.as_ref().expect("必须返回诊断");
    assert_eq!(error.error.code, LogicErrorCode::InvalidSyntax);
    assert_eq!(inspection.line, Some(error.line));
    assert_eq!(inspection.column, Some(error.column));
    assert!(
        inspection
            .error_offset
            .is_some_and(|offset| offset <= source.len())
    );
}

#[test]
fn 共享格式化样例逐字节一致且二次解析与格式化幂等() {
    // 与前端同读 texts.json 的 format_cases；expected 为 TS 真实输出的字节快照。
    let fixture = texts_fixture();
    let flows = flows_fixture();
    for item in fixture["format_cases"].as_array().unwrap() {
        let name = item["name"].as_str().unwrap();
        let questions: Vec<Question> =
            serde_json::from_value(format_case_field(item, &flows, "questions").clone()).unwrap();
        let logic: DocumentLogic =
            serde_json::from_value(format_case_field(item, &flows, "logic").clone()).unwrap();
        let expected = item["expected"].as_str().unwrap();
        let formatted = format_logic_text(&logic, &questions)
            .unwrap_or_else(|error| panic!("{name} 格式化失败: {error:?}"));
        // 逐字节对齐 TS formatLogicText 输出。
        assert_eq!(formatted, expected, "{name} 输出与共享期望不一致");

        let once = parse_logic_text(&formatted, &questions)
            .unwrap_or_else(|error| panic!("{name} 一次解析失败: {error:?}"));
        let reformatted = format_logic_text(&once, &questions)
            .unwrap_or_else(|error| panic!("{name} 二次格式化失败: {error:?}"));
        assert_eq!(reformatted, formatted, "{name} 二次格式化必须幂等");

        let twice = parse_logic_text(&reformatted, &questions)
            .unwrap_or_else(|error| panic!("{name} 二次解析失败: {error:?}"));
        // DocumentLogic 已实现 PartialEq，直接比较两次解析结果。
        assert_eq!(once, twice, "{name} 二次解析与一次解析不等值");
    }
}

#[test]
fn 三元条件作为二元操作数时加括号并往返一致() {
    // 左右操作数都必须是三元条件：否则 else 分支会吞掉相邻运算符。
    let logic: DocumentLogic = serde_json::from_value(json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "variables": [
            {
                "id": "00000000-0000-4000-8000-000000004001",
                "value_type": "number",
                "expression": {
                    "op": "arithmetic",
                    "operator": "add",
                    "left": {
                        "op": "if",
                        "condition": {"op": "boolean", "value": true},
                        "then_branch": {"op": "number", "value": "1"},
                        "else_branch": {"op": "number", "value": "2"}
                    },
                    "right": {"op": "number", "value": "3"}
                }
            },
            {
                "id": "00000000-0000-4000-8000-000000004002",
                "value_type": "number",
                "expression": {
                    "op": "arithmetic",
                    "operator": "subtract",
                    "left": {"op": "number", "value": "9"},
                    "right": {
                        "op": "if",
                        "condition": {"op": "boolean", "value": false},
                        "then_branch": {"op": "number", "value": "1"},
                        "else_branch": {"op": "number", "value": "2"}
                    }
                }
            },
            {
                "id": "00000000-0000-4000-8000-000000004003",
                "value_type": "boolean",
                "expression": {
                    "op": "compare",
                    "comparison": "eq",
                    "left": {
                        "op": "if",
                        "condition": {"op": "boolean", "value": true},
                        "then_branch": {"op": "boolean", "value": true},
                        "else_branch": {"op": "boolean", "value": false}
                    },
                    "right": {"op": "boolean", "value": true}
                }
            }
        ],
        "rules": []
    }))
    .unwrap();
    let formatted = format_logic_text(&logic, &[]).unwrap();
    assert!(
        formatted.contains("((if true then number(\"1\") else number(\"2\")) + number(\"3\"))"),
        "{formatted}"
    );
    assert!(
        formatted.contains("(number(\"9\") - (if false then number(\"1\") else number(\"2\")))"),
        "{formatted}"
    );
    assert!(
        formatted.contains("((if true then true else false) == true)"),
        "{formatted}"
    );
    // 往返后必须得到同一棵 AST，而不是分支被相邻运算符改写。
    assert_eq!(parse_logic_text(&formatted, &[]).unwrap(), logic);
}

#[test]
fn 并列三元条件不触发解析嵌套预算且往返一致() {
    // 63 个并列 if 只占 `all` 的参数层，不得按语句累加成 63 层嵌套。
    let expressions: Vec<Value> = (0..63)
        .map(|_| {
            json!({
                "op": "if",
                "condition": {"op": "boolean", "value": true},
                "then_branch": {"op": "boolean", "value": true},
                "else_branch": {"op": "boolean", "value": false}
            })
        })
        .collect();
    let logic: DocumentLogic = serde_json::from_value(json!({
        "dsl_version": 1,
        "semantics_version": 1,
        "variables": [{
            "id": "00000000-0000-4000-8000-000000004004",
            "value_type": "boolean",
            "expression": {"op": "all", "expressions": expressions}
        }],
        "rules": []
    }))
    .unwrap();
    let formatted = format_logic_text(&logic, &[]).unwrap();
    assert_eq!(parse_logic_text(&formatted, &[]).unwrap(), logic);
}

#[test]
fn 绑定校验按源码顺序定位到具体语句() {
    // 第 2 行绑定了不存在的题目，错误必须定位到该 bind 行而不是头部。
    let source = concat!(
        "dsl 1 semantics 1;\n",
        "bind question ghost = \"00000000-0000-4000-8000-0000000000ff\";\n",
    );
    let error = parse_program(source, &[]).unwrap_err();
    assert_eq!(error.error.code, LogicErrorCode::InvalidReference);
    assert_eq!(error.line, 2);
    assert_eq!(error.column, 1);
    let offset = source.find("bind question ghost").unwrap();
    assert_eq!(inspect_logic_text(source, &[]).error_offset, Some(offset));
}

#[test]
fn 数字字面量原文超过字节上限时拒绝() {
    // 数字字面量原文最多 64 字节，边界值仍可编译，超长原文按无效数字拒绝。
    let declaration = |literal: &str| {
        format!(
            "dsl 1 semantics 1;\n\
             bind variable v = \"00000000-0000-4000-8000-000000006001\";\n\
             let v: number = number(\"{literal}\");"
        )
    };
    let boundary = "1".repeat(64);
    assert!(
        parse_program(&declaration(&boundary), &[]).is_ok(),
        "64 字节的原文仍在预算内"
    );
    let error = parse_program(&declaration(&"1".repeat(65)), &[]).unwrap_err();
    assert_eq!(error.error.code, LogicErrorCode::InvalidNumber);
    assert_eq!(error.line, 3);
}

#[test]
fn 题目与选项身份撞车时拒绝格式化() {
    // 第二个题目的身份与第一个题目的选项相同，生成别名时不得静默覆盖。
    let questions: Vec<Question> = serde_json::from_value(json!([
        {
            "id": "00000000-0000-4000-8000-000000005001",
            "title": "选项",
            "required": false,
            "type": "single_choice",
            "options": [{"id": "00000000-0000-4000-8000-000000005002", "label": "甲"}]
        },
        {
            "id": "00000000-0000-4000-8000-000000005002",
            "title": "碰撞",
            "required": false,
            "type": "text",
            "max_length": 10
        }
    ]))
    .unwrap();
    let logic = DocumentLogic {
        dsl_version: 1_u32.into(),
        semantics_version: 1_u32.into(),
        variables: Vec::new(),
        rules: Vec::new(),
        flow: None,
    };
    let error = format_logic_text(&logic, &questions).unwrap_err();
    assert_eq!(error.code, LogicErrorCode::InvalidReference);
    assert_eq!(error.path, "$");
}
