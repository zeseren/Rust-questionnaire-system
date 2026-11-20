//! 共享夹具读取：`fragments` 里的命名片段按引用展开，未知引用立即失败。
#![allow(dead_code)]

use serde_json::Value;

fn parse(raw: &str) -> Value {
    serde_json::from_str(raw).expect("夹具必须是合法 JSON")
}

/// 展开命名片段：`questions`/`variables`/`rules` 数组中的字符串按名称取片段。
fn expand(value: &mut Value, fragments: &Value) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields.iter_mut() {
                if matches!(key.as_str(), "questions" | "variables" | "rules")
                    && let Value::Array(items) = child
                {
                    for item in items.iter_mut() {
                        if let Value::String(name) = item {
                            let target = fragments
                                .get(name.as_str())
                                .unwrap_or_else(|| panic!("未知夹具片段：{name}"));
                            *item = target.clone();
                        }
                    }
                }
                expand(child, fragments);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| expand(item, fragments)),
        _ => {}
    }
}

fn expanded(raw: &str) -> Value {
    let mut fixture = parse(raw);
    let fragments = fixture.get("fragments").cloned().unwrap_or(Value::Null);
    expand(&mut fixture, &fragments);
    fixture
}

pub fn flows() -> Value {
    expanded(include_str!("../../../fixtures/survey-logic/flows.json"))
}

/// 按名称取已展开的流程文档；未知名称立即失败。
pub fn flow_document(name: &str) -> Value {
    let documents = flows();
    documents["documents"]
        .get(name)
        .unwrap_or_else(|| panic!("未知流程文档：{name}"))
        .clone()
}

pub fn texts() -> Value {
    parse(include_str!("../../../fixtures/survey-logic/texts.json"))
}

pub fn documents() -> Value {
    expanded(include_str!(
        "../../../fixtures/survey-logic/documents.json"
    ))
}

pub fn expressions() -> Value {
    parse(include_str!(
        "../../../fixtures/survey-logic/expressions.json"
    ))
}

pub fn submissions() -> Value {
    parse(include_str!(
        "../../../fixtures/survey-logic/submissions.json"
    ))
}

pub fn input_contracts() -> Value {
    parse(include_str!(
        "../../../fixtures/survey-logic/input-contracts.json"
    ))
}
