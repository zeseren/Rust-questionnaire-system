//! 文本 DSL 回归：共享人工期望、格式化往返幂等、文法负例与函数签名校验。

use questionnaire_logic::domain::{
    Document, DocumentLogic, LogicErrorCode, LogicTextError, Question, format_logic_text,
    parse_logic_text,
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
fn 共享格式化样例逐字节一致且二次解析与格式化幂等() {
    // 与前端同读 texts.json 的 format_cases；expected 为 TS 真实输出的字节快照。
    let fixture = texts_fixture();
    for item in fixture["format_cases"].as_array().unwrap() {
        let name = item["name"].as_str().unwrap();
        let questions: Vec<Question> = serde_json::from_value(item["questions"].clone()).unwrap();
        let logic: DocumentLogic = serde_json::from_value(item["logic"].clone()).unwrap();
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
