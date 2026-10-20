//! 配置、作答及选项身份统一只接受非零、小写连字符 UUID。

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::{DeserializeSeed, Deserializer, Error, MapAccess, SeqAccess, Visitor};

/// 身份类型统一为 `uuid::Uuid`；一并导出，下游无需重复声明同一依赖版本。
pub use uuid::Uuid;

/// 身份文本校验失败时对外的统一说明。
const INVALID_ID: &str = "身份 ID 必须是非零的小写连字符 UUID";

/// 严格解析身份文本：只接受 36 字符、非空、全小写连字符 UUID。
pub fn parse(raw: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(raw).ok().filter(|id| !id.is_nil())?;
    (raw.len() == 36
        && raw
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f' | b'-')))
    .then_some(id)
}

/// 单个身份的访问器：直接借用输入 `&str` 校验，不分配中间 `String`。
struct IdVisitor;

impl<'de> Visitor<'de> for IdVisitor {
    type Value = Uuid;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("非零的小写连字符 UUID 字符串")
    }

    fn visit_str<E: Error>(self, raw: &str) -> Result<Self::Value, E> {
        parse(raw).ok_or_else(|| E::custom(INVALID_ID))
    }
}

/// 序列元素的身份种子：`SeqAccess` 按元素逐个复用同一份校验逻辑。
struct IdSeed;

impl<'de> DeserializeSeed<'de> for IdSeed {
    type Value = Uuid;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserialize(deserializer)
    }
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    deserializer.deserialize_str(IdVisitor)
}

pub fn deserialize_vec<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Uuid>, D::Error> {
    struct IdentitiesVisitor;

    impl<'de> Visitor<'de> for IdentitiesVisitor {
        type Value = Vec<Uuid>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("非零的小写连字符 UUID 数组")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let mut ids = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
            while let Some(id) = sequence.next_element_seed(IdSeed)? {
                ids.push(id);
            }
            Ok(ids)
        }
    }

    deserializer.deserialize_seq(IdentitiesVisitor)
}

/// 反序列化以题目身份为键的对象；键与单值身份使用同一套严格校验。
pub fn deserialize_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<Uuid, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct AnswersVisitor<V>(PhantomData<V>);

    impl<'de, V: Deserialize<'de>> Visitor<'de> for AnswersVisitor<V> {
        type Value = BTreeMap<Uuid, V>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("以题目 UUID 为键的答案对象")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut values = BTreeMap::new();
            while let Some((id, value)) = map.next_entry_seed(IdSeed, PhantomData::<V>)? {
                values.insert(id, value);
            }
            Ok(values)
        }
    }

    deserializer.deserialize_map(AnswersVisitor(PhantomData))
}
