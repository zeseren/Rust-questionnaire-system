//! 配置、作答及选项身份统一只接受非零的标准连字符 UUID；十六进制大小写等价，输出归一为小写。

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::{Deserializer, Error};

/// 身份类型统一为 `uuid::Uuid`；一并导出，下游无需重复声明同一依赖版本。
pub use uuid::Uuid;

/// 身份文本校验失败时对外的统一说明。
pub(crate) const INVALID_ID: &str = "身份 ID 必须是非零的标准连字符 UUID";

/// 解析身份文本：只接受 36 字符的标准连字符格式（十六进制大小写均可），并拒绝 Nil UUID。
///
/// 花括号、URN、无连字符等其余 RFC 4122 写法虽然能被 `uuid` 解析，但不是本项目的契约格式；
/// 解析结果统一为 `Uuid`，序列化输出仍为小写连字符。
#[must_use]
pub fn parse(raw: &str) -> Option<Uuid> {
    if raw.len() != 36
        || !raw
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' | b'-'))
    {
        return None;
    }
    Uuid::parse_str(raw).ok().filter(|id| !id.is_nil())
}

/// 反序列化单个身份字符串，并复用严格解析规则。
///
/// # Errors
/// 输入不是字符串，或不是非零的标准连字符 UUID 时返回反序列化错误。
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    let raw = String::deserialize(deserializer)
        .map_err(|_| D::Error::custom("非零的标准连字符 UUID 字符串"))?;
    parse(&raw).ok_or_else(|| D::Error::custom(INVALID_ID))
}

/// 反序列化身份字符串数组，逐元素复用严格解析规则。
///
/// # Errors
/// 输入不是字符串数组，或任一元素不是非零的标准连字符 UUID 时返回反序列化错误。
pub fn deserialize_vec<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Uuid>, D::Error> {
    Vec::<String>::deserialize(deserializer)
        .map_err(|_| D::Error::custom("非零的标准连字符 UUID 数组"))?
        .iter()
        .map(|raw| parse(raw).ok_or_else(|| D::Error::custom(INVALID_ID)))
        .collect()
}

/// 反序列化以题目身份为键的对象；键与单值身份使用同一套严格校验。
///
/// # Errors
/// 输入不是对象，或任一键不是非零的标准连字符 UUID 时返回反序列化错误。
pub fn deserialize_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<Uuid, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    BTreeMap::<String, V>::deserialize(deserializer)
        .map_err(|_| D::Error::custom("以题目 UUID 为键的答案对象"))?
        .into_iter()
        .map(|(raw, value)| {
            parse(&raw)
                .map(|id| (id, value))
                .ok_or_else(|| D::Error::custom(INVALID_ID))
        })
        .collect()
}
