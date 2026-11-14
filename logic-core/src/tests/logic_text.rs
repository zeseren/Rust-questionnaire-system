//! 文本 DSL 内部不变量：字节偏移行列换算、词法预算预检与巡检诊断输出。

use crate::domain::{Question, QuestionKind};
use crate::identity::Uuid;
use crate::logic::{LogicErrorCode, error};
use crate::logic_text::{check_text_budget, inspect_logic_text, located};

fn 位置(source: &str, offset: usize) -> (usize, usize) {
    let 错误 = located(
        source,
        offset,
        error(LogicErrorCode::InvalidSyntax, "$", None),
    );
    (错误.line, 错误.column)
}

#[test]
fn 定位按字节偏移换算行列() {
    // LF 与 CRLF 的行号都只统计 `\n`，列号从 1 开始。
    assert_eq!(位置("abc\ndef", 0), (1, 1));
    assert_eq!(位置("abc\ndef", 3), (1, 4));
    assert_eq!(位置("abc\ndef", 4), (2, 1));
    assert_eq!(位置("abc\ndef", 6), (2, 3));
    assert_eq!(位置("ab\r\ncd", 4), (2, 1));
    assert_eq!(位置("ab\r\ncd", 6), (2, 3));

    // 列号按 Unicode 标量计数，中文占三字节、Emoji 占四字节但都只算一列。
    assert_eq!(位置("中文😀x", 10), (1, 4));
    assert_eq!(位置("a\n中文😀", 12), (2, 4));
}

#[test]
fn 文本预算超限错误定位到精确行列() {
    // 字节预算：多一个字节立即拒绝并定位到首行首列。
    let 错误 = check_text_budget(&" ".repeat(1_048_577)).unwrap_err();
    assert_eq!(错误.error.code, LogicErrorCode::LimitExceeded);
    assert_eq!((错误.line, 错误.column), (1, 1));

    // 64 层括号使深度超限，必须定位到第 64 个左括号。
    let 超限 = format!("{}true{}", "(".repeat(64), ")".repeat(64));
    let 错误 = check_text_budget(&超限).unwrap_err();
    assert_eq!(错误.error.code, LogicErrorCode::LimitExceeded);
    assert_eq!((错误.line, 错误.column), (1, 64));

    // 前缀 `!` 与括号共用同一深度预算，同样定位到第 64 个字符。
    let 错误 = check_text_budget(&format!("{}true", "!".repeat(64))).unwrap_err();
    assert_eq!(错误.error.code, LogicErrorCode::LimitExceeded);
    assert_eq!((错误.line, 错误.column), (1, 64));

    // 前缀一元负号与 `!` 使用同一预算；二元减号不计入前缀深度。
    let 错误 = check_text_budget(&format!("{}true", "-".repeat(64))).unwrap_err();
    assert_eq!(错误.error.code, LogicErrorCode::LimitExceeded);
    assert_eq!((错误.line, 错误.column), (1, 64));
    assert!(
        check_text_budget(&format!("1{}", " - 1".repeat(64))).is_ok(),
        "连续的二元减号不构成前缀嵌套"
    );
}

#[test]
fn 巡检在后续语法错误时仍返回已扫描绑定() {
    let source = concat!(
        "dsl 1 semantics 1;\n",
        "// 注释中的 bind 不算绑定\n",
        "bind question q = \"00000000-0000-4000-8000-000000000001\";\n",
        "bind option o = \"00000000-0000-4000-8000-000000000101\";\n",
        "let broken = ;",
    );
    let inspection = inspect_logic_text(source, &[]);
    assert_eq!(inspection.bindings.len(), 2, "注释与字符串不影响扫描");
    assert_eq!(inspection.bindings[0].category, "question");
    assert_eq!(inspection.bindings[0].alias, "q");
    assert_eq!(
        inspection.bindings[0].id,
        Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap()
    );
    assert_eq!(inspection.bindings[1].category, "option");
    let error = inspection.error.as_ref().expect("语法错误必须返回");
    assert_eq!(error.error.code, LogicErrorCode::InvalidSyntax);
    assert_eq!(inspection.line, Some(error.line));
    assert_eq!(inspection.column, Some(error.column));
    assert!(
        inspection
            .error_offset
            .is_some_and(|offset| offset <= source.len()),
        "错误偏移必须落在源码范围内"
    );

    // 完整解析成功时不携带诊断。
    let question = Question {
        id: Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap(),
        title: "文本".into(),
        required: false,
        kind: QuestionKind::Text { max_length: 10 },
    };
    let source = concat!(
        "dsl 1 semantics 1;\n",
        "bind question q = \"00000000-0000-4000-8000-000000000001\";\n",
        "visible q = true;",
    );
    let inspection = inspect_logic_text(source, &[question]);
    assert!(inspection.error.is_none(), "合法文本不应报错");
    assert_eq!(inspection.bindings.len(), 1);
}
