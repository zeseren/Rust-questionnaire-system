//! 身份解析与反序列化的模块内不变量测试。

use std::collections::BTreeMap;

use serde::Deserialize;
use uuid::Uuid;

use crate::identity::{INVALID_ID, parse};

/// 合法的非零连字符 UUID。
const 合法: &str = "00000000-0000-0000-0000-000000000001";
const 合法二: &str = "00000000-0000-0000-0000-000000000002";

#[derive(Debug, Deserialize)]
struct 单值 {
    #[serde(deserialize_with = "crate::identity::deserialize")]
    id: Uuid,
}

#[derive(Debug, Deserialize)]
struct 数组 {
    #[serde(deserialize_with = "crate::identity::deserialize_vec")]
    ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
struct 映射 {
    #[serde(deserialize_with = "crate::identity::deserialize_map")]
    answers: BTreeMap<Uuid, u32>,
}

#[test]
fn 解析接受十六进制大小写但拒绝非标准格式与零值() {
    assert_eq!(parse(合法), Some(Uuid::from_u128(1)));
    assert_eq!(
        parse("ffffffff-ffff-ffff-ffff-ffffffffffff"),
        Some(Uuid::from_u128(u128::MAX))
    );

    // 大小写十六进制等价，解析结果统一为同一 `Uuid`。
    assert_eq!(
        parse("AAAAAAAa-AAAA-4AAA-8AAA-AAAAAAAAAAAA"),
        Some(Uuid::from_u128(0xaaaa_aaaa_aaaa_4aaa_8aaa_aaaa_aaaa_aaaa)),
        "大写十六进制是合法 UUID"
    );
    assert_eq!(
        parse("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF"),
        Some(Uuid::from_u128(u128::MAX))
    );

    for (原文, 说明) in [
        (
            "00000000-0000-0000-0000-000000000000",
            "Nil UUID 不是有效身份",
        ),
        ("{00000000-0000-4000-8000-000000000001}", "花括号包裹"),
        ("urn:uuid:00000000-0000-4000-8000-000000000001", "URN 前缀"),
        ("00000000-0000-4000-8000-00000000000g", "非法十六进制字符"),
        ("00000000_0000_4000_8000_000000000001", "下划线分隔"),
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

    let 错误 = serde_json::from_str::<单值>(r#"{"id":"00000000-0000-0000-0000-000000000000"}"#)
        .unwrap_err();
    assert!(错误.to_string().contains(INVALID_ID), "{错误}");

    let 错误 = serde_json::from_str::<单值>(r#"{"id":1}"#).unwrap_err();
    assert!(
        错误.to_string().contains("非零的标准连字符 UUID 字符串"),
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
        错误.to_string().contains("非零的标准连字符 UUID 数组"),
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

    let 错误 =
        serde_json::from_str::<映射>(r#"{"answers":{"00000000-0000-0000-0000-000000000000":1}}"#)
            .unwrap_err();
    assert!(
        错误.to_string().contains(INVALID_ID),
        "非法键应被拒绝：{错误}"
    );
}
