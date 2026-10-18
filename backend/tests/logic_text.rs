//! 两端使用同一文本和人工期望，保证别名绑定、数值原文与诊断位置一致。
use questionnaire_api::domain::{Document, LogicErrorCode, format_logic_text, parse_logic_text};
use serde_json::{Value, json};

#[test]
fn 文本共享样例保留身份优先级并定位语法与语义错误() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/texts.json")).unwrap();
    let flows: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/flows.json")).unwrap();
    let document: Document =
        serde_json::from_value(flows["documents"][fixture["document"].as_str().unwrap()].clone())
            .unwrap();
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
fn 文本与表达式预算允许边界并拒绝长链和混合嵌套超限() {
    let prefix = "dsl 1 semantics 1; bind variable v = \"00000000-0000-4000-8000-000000002001\"; let v: boolean = ";
    for expression in [
        format!("{}true{}", "(".repeat(63), ")".repeat(63)),
        format!("!{}true{}", "(".repeat(62), ")".repeat(62)),
        format!("all({}true{})", "(".repeat(62), ")".repeat(62)),
    ] {
        assert!(parse_logic_text(&format!("{prefix}{expression};"), &[]).is_ok());
    }
    for expression in [
        format!("{}true{}", "(".repeat(64), ")".repeat(64)),
        format!("!all({}true{})", "(".repeat(62), ")".repeat(62)),
    ] {
        let error = parse_logic_text(&format!("{prefix}{expression};"), &[]).unwrap_err();
        assert_eq!(error.error.code, LogicErrorCode::LimitExceeded);
    }
    assert_eq!(
        parse_logic_text(&" ".repeat(1_048_577), &[])
            .unwrap_err()
            .error
            .code,
        LogicErrorCode::LimitExceeded
    );
}

#[test]
fn 内置函数检查参数元数与引用种类() {
    let prefix = "dsl 1 semantics 1; bind variable v = \"00000000-0000-4000-8000-000000002001\"; let v: number = ";
    for expression in ["number(1)", "coalesce(1)", "length([])"] {
        let error = parse_logic_text(&format!("{prefix}{expression};"), &[]).unwrap_err();
        assert_eq!(error.error.code, LogicErrorCode::InvalidSyntax);
    }
}

#[test]
fn 共享格式化样例逐字节一致且二次解析等值() {
    // 与前端同读 texts.json 的 format_cases；expected 为 TS 真实输出的字节快照。
    let fixture: Value =
        serde_json::from_str(include_str!("../../fixtures/survey-logic/texts.json")).unwrap();
    for item in fixture["format_cases"].as_array().unwrap() {
        let name = item["name"].as_str().unwrap();
        let questions: Vec<questionnaire_api::domain::Question> =
            serde_json::from_value(item["questions"].clone()).unwrap();
        let logic: questionnaire_api::domain::DocumentLogic =
            serde_json::from_value(item["logic"].clone()).unwrap();
        let expected = item["expected"].as_str().unwrap();
        let formatted = format_logic_text(&logic, &questions)
            .unwrap_or_else(|error| panic!("{name} 格式化失败: {error:?}"));
        // 逐字节对齐 TS formatLogicText 输出。
        assert_eq!(formatted, expected, "{name} 输出与共享期望不一致");
        let once = parse_logic_text(&formatted, &questions)
            .unwrap_or_else(|error| panic!("{name} 一次解析失败: {error:?}"));
        let twice = parse_logic_text(
            &format_logic_text(&once, &questions)
                .unwrap_or_else(|error| panic!("{name} 二次格式化失败: {error:?}")),
            &questions,
        )
        .unwrap_or_else(|error| panic!("{name} 二次解析失败: {error:?}"));
        // DocumentLogic 已实现 PartialEq，直接比较两次解析结果。
        assert_eq!(once, twice, "{name} 二次解析与一次解析不等值");
    }
}
