//! 配置、作答及选项身份统一只接受非零、小写连字符 UUID。

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::{Deserializer, Error};

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

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    let raw = String::deserialize(deserializer)
        .map_err(|_| D::Error::custom("非零的小写连字符 UUID 字符串"))?;
    parse(&raw).ok_or_else(|| D::Error::custom(INVALID_ID))
}

pub fn deserialize_vec<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Uuid>, D::Error> {
    Vec::<String>::deserialize(deserializer)
        .map_err(|_| D::Error::custom("非零的小写连字符 UUID 数组"))?
        .iter()
        .map(|raw| parse(raw).ok_or_else(|| D::Error::custom(INVALID_ID)))
        .collect()
}

/// 反序列化以题目身份为键的对象；键与单值身份使用同一套严格校验。
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde::Deserialize;
    use uuid::Uuid;

    use super::{INVALID_ID, parse};

    /// 合法的非零小写连字符 UUID。
    const 合法: &str = "00000000-0000-0000-0000-000000000001";
    const 合法二: &str = "00000000-0000-0000-0000-000000000002";

    #[derive(Debug, Deserialize)]
    struct 单值 {
        #[serde(deserialize_with = "super::deserialize")]
        id: Uuid,
    }

    #[derive(Debug, Deserialize)]
    struct 数组 {
        #[serde(deserialize_with = "super::deserialize_vec")]
        ids: Vec<Uuid>,
    }

    #[derive(Debug, Deserialize)]
    struct 映射 {
        #[serde(deserialize_with = "super::deserialize_map")]
        answers: BTreeMap<Uuid, u32>,
    }

    #[test]
    fn 严格解析只接受非零小写连字符身份() {
        assert_eq!(parse(合法), Some(Uuid::from_u128(1)));
        assert_eq!(
            parse("ffffffff-ffff-ffff-ffff-ffffffffffff"),
            Some(Uuid::from_u128(u128::MAX))
        );

        for (原文, 说明) in [
            (
                "00000000-0000-0000-0000-000000000000",
                "Nil UUID 不是有效身份",
            ),
            ("00000000-0000-4000-8000-00000000000A", "含大写十六进制字母"),
            ("{00000000-0000-4000-8000-000000000001}", "花括号包裹"),
            ("urn:uuid:00000000-0000-4000-8000-000000000001", "URN 前缀"),
            ("00000000-0000-4000-8000-00000000000g", "非法十六进制字符"),
            (" 00000000-0000-4000-8000-000000000001", "含首部空白"),
            ("00000000-0000-4000-8000-000000000001 ", "含尾部空白"),
        ] {
            assert_eq!(parse(原文), None, "应拒绝{说明}：{原文}");
        }

        let 无连字符 = 合法.replace('-', "");
        assert_eq!(parse(&无连字符), None, "32 字符无连字符应拒绝");
        assert_eq!(parse(&合法[..35]), None, "35 字符应拒绝");
        assert_eq!(parse(&format!("{合法}0")), None, "37 字符应拒绝");
    }

    #[test]
    fn 单值反序列化复用严格解析文案() {
        let 值: 单值 = serde_json::from_str(&format!(r#"{{"id":"{合法}"}}"#)).unwrap();
        assert_eq!(值.id, Uuid::from_u128(1));

        let 错误 =
            serde_json::from_str::<单值>(r#"{"id":"00000000-0000-0000-0000-000000000000"}"#)
                .unwrap_err();
        assert!(错误.to_string().contains(INVALID_ID), "{错误}");

        let 错误 = serde_json::from_str::<单值>(r#"{"id":1}"#).unwrap_err();
        assert!(
            错误.to_string().contains("非零的小写连字符 UUID 字符串"),
            "非字符串必须给出期望类型：{错误}"
        );
    }

    #[test]
    fn 数组反序列化逐元素校验并原样保留重复() {
        let 空: 数组 = serde_json::from_str(r#"{"ids":[]}"#).unwrap();
        assert!(空.ids.is_empty());

        let ids: 数组 = serde_json::from_str(&format!(r#"{{"ids":["{合法}","{合法}"]}}"#)).unwrap();
        assert_eq!(ids.ids, vec![Uuid::from_u128(1), Uuid::from_u128(1)]);

        let 错误 = serde_json::from_str::<数组>(&format!(
            r#"{{"ids":["{合法}","00000000-0000-0000-0000-000000000000"]}}"#
        ))
        .unwrap_err();
        assert!(
            错误.to_string().contains(INVALID_ID),
            "非法元素应被拒绝：{错误}"
        );

        let 错误 = serde_json::from_str::<数组>(r#"{"ids":{}}"#).unwrap_err();
        assert!(
            错误.to_string().contains("非零的小写连字符 UUID 数组"),
            "非数组必须给出期望类型：{错误}"
        );
    }

    #[test]
    fn 映射反序列化校验键身份并按身份排序() {
        let 答案: 映射 = serde_json::from_str(&format!(r#"{{"answers":{{"{合法}":7}}}}"#)).unwrap();
        assert_eq!(答案.answers.get(&Uuid::from_u128(1)), Some(&7));

        let 多个: 映射 =
            serde_json::from_str(&format!(r#"{{"answers":{{"{合法二}":2,"{合法}":1}}}}"#)).unwrap();
        assert_eq!(
            多个.answers.keys().copied().collect::<Vec<_>>(),
            vec![Uuid::from_u128(1), Uuid::from_u128(2)]
        );

        let 错误 = serde_json::from_str::<映射>(r#"{"answers":[]}"#).unwrap_err();
        assert!(
            错误.to_string().contains("以题目 UUID 为键的答案对象"),
            "非对象必须给出期望类型：{错误}"
        );

        let 错误 = serde_json::from_str::<映射>(
            r#"{"answers":{"00000000-0000-0000-0000-000000000000":1}}"#,
        )
        .unwrap_err();
        assert!(
            错误.to_string().contains(INVALID_ID),
            "非法键应被拒绝：{错误}"
        );
    }
}
