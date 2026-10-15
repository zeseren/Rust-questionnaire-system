//! 配置及作答的题目身份只接受非零、小写连字符 UUID；答案值由领域层单独规范化。

use serde::{Deserialize, Deserializer, de::Error};
use uuid::Uuid;

pub(crate) fn parse(raw: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(raw).ok().filter(|id| !id.is_nil())?;
    (raw.len() == 36
        && raw
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f' | b'-')))
    .then_some(id)
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    let raw = String::deserialize(deserializer)?;
    parse(&raw).ok_or_else(|| D::Error::custom("身份 ID 必须是非零的小写连字符 UUID"))
}

pub(crate) fn deserialize_vec<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Uuid>, D::Error> {
    #[derive(Deserialize)]
    struct Identity(#[serde(deserialize_with = "deserialize")] Uuid);

    Vec::<Identity>::deserialize(deserializer).map(|ids| ids.into_iter().map(|id| id.0).collect())
}
