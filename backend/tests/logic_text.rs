//! 两端使用同一文本和人工期望，保证别名绑定、数值原文与诊断位置一致。
use questionnaire_api::domain::{Document, LogicErrorCode, parse_logic_text};
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
        format!("{}1", "1 + ".repeat(20_000)),
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
fn 负号只接数字字面量且函数不能放宽参数种类() {
    let prefix = "dsl 1 semantics 1; bind variable v = \"00000000-0000-4000-8000-000000002001\"; let v: number = ";
    for expression in [
        "-(1)",
        "-length(\"文\")",
        "number(1)",
        "coalesce(1)",
        "length([])",
    ] {
        let error = parse_logic_text(&format!("{prefix}{expression};"), &[]).unwrap_err();
        assert_eq!(error.error.code, LogicErrorCode::InvalidSyntax);
    }
}
