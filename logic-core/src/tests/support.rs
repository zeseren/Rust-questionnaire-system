//! 测试共用构造：身份、规则包版本、选项与数字题的最小样板。

use serde_json::Number;
use uuid::Uuid;

use crate::domain::{Choice, Question, QuestionKind};

/// 测试身份：小整数直接映射为 UUID。
pub(crate) fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

/// 规则包版本：当前只接受 1。
pub(crate) fn 版本() -> Number {
    Number::from(1_u32)
}

/// 选择题选项；文字随身份变化，断言失败时便于定位。
pub(crate) fn 选项(value: u128) -> Choice {
    Choice {
        id: id(value),
        label: format!("选项 {value}"),
    }
}

/// 数字题；整数性与上下界由用例指定。
pub(crate) fn 数字题(
    value: u128,
    integer: bool,
    min: Option<f64>,
    max: Option<f64>,
) -> Question {
    Question {
        id: id(value),
        title: format!("数字题 {value}"),
        required: false,
        kind: QuestionKind::Number { integer, min, max },
    }
}
