//! 问卷状态机、回收资格与公开提交契约：全部为纯函数，不触碰数据库。

use std::collections::BTreeMap;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use sea_orm::ActiveEnum;
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    domain::Document,
    entities::{AuditAction, FillingMode, SurveyStatus, UserRole},
    error::AppError,
    repository::{SurveyMetaRow, ensure_collecting},
    submissions::{
        idempotency_conflict, privacy_notice, submission_payload_hash, validate_idempotency_key,
    },
    surveys::{
        CollectionAction, CollectionSettings, SurveyFilters, ensure_editable,
        ensure_future_deadline, ensure_revision, next_collection_status, survey_detail,
        validate_mode_change,
    },
    tests::support::{error_json, survey_model},
};

async fn 三元组(error: AppError) -> (StatusCode, String, String) {
    let (状态, _, body) = error_json(error).await;
    (
        状态,
        body["error"]["code"]
            .as_str()
            .expect("错误码必须是字符串")
            .to_owned(),
        body["error"]["message"]
            .as_str()
            .expect("错误消息必须是字符串")
            .to_owned(),
    )
}

/// 断言冲突错误的状态码、错误码与消息。
async fn 断言冲突(error: AppError, 期望码: &str, 期望消息: &str) {
    let (状态, 码, 消息) = 三元组(error).await;
    assert_eq!(状态, StatusCode::CONFLICT, "{期望码}");
    assert_eq!(码, 期望码);
    assert_eq!(消息, 期望消息);
}

#[test]
fn 问卷状态在截止时间边界上物化() {
    let now = Utc::now();
    let 未来 = now + Duration::seconds(1);
    let 过去 = now - Duration::seconds(1);
    for (status, deadline, 期望) in [
        (SurveyStatus::Collecting, None, SurveyStatus::Collecting),
        (
            SurveyStatus::Collecting,
            Some(未来),
            SurveyStatus::Collecting,
        ),
        (SurveyStatus::Collecting, Some(now), SurveyStatus::Closed),
        (SurveyStatus::Collecting, Some(过去), SurveyStatus::Closed),
        (SurveyStatus::Closed, None, SurveyStatus::Closed),
        (SurveyStatus::Closed, Some(未来), SurveyStatus::Closed),
        (
            SurveyStatus::Unpublished,
            Some(过去),
            SurveyStatus::Unpublished,
        ),
        (SurveyStatus::Archived, Some(过去), SurveyStatus::Archived),
    ] {
        assert_eq!(
            status.effective(deadline, now),
            期望,
            "{status:?} 在 {deadline:?} 到期时必须映射为 {期望:?}"
        );
    }
}

#[test]
fn 实体枚举与数据库检查和序列化值严格对齐() {
    /// 从 schema.sql 的 CHECK 约束解析允许的文本集合。
    fn 约束值集合(列名: &str) -> std::collections::BTreeSet<String> {
        let 前缀 = format!("{列名} IN (");
        let 行 = include_str!("../schema.sql")
            .lines()
            .find(|行| 行.contains(&前缀))
            .expect("schema.sql 必须包含该列的 CHECK 约束");
        let 开始 = 行.find(&前缀).expect("前缀已匹配") + 前缀.len();
        let 结束 = 开始 + 行[开始..].find(')').expect("CHECK 约束必须以右括号结束");
        行[开始..结束]
            .split(',')
            .map(|文本| 文本.trim().trim_matches('\'').to_owned())
            .collect()
    }

    fn 断言枚举<T: ActiveEnum<Value = String> + Serialize>(变体: &[(T, &str)], 列名: &str) {
        let 期望: std::collections::BTreeSet<String> =
            变体.iter().map(|(_, 文本)| (*文本).to_owned()).collect();
        assert_eq!(
            约束值集合(列名),
            期望,
            "{列名} 的 CHECK 约束变化时必须同步枚举定义"
        );
        for (值, 期望文本) in 变体 {
            let 数据库值 = 值.to_value();
            let 序列化值 = serde_json::to_value(值).expect("枚举必须可序列化");
            assert_eq!(
                Value::String(数据库值.clone()),
                序列化值,
                "SeaORM 与 serde 的字符串值必须一致"
            );
            assert_eq!(数据库值.as_str(), *期望文本);
        }
    }

    断言枚举(
        &[
            (SurveyStatus::Unpublished, "unpublished"),
            (SurveyStatus::Collecting, "collecting"),
            (SurveyStatus::Closed, "closed"),
            (SurveyStatus::Archived, "archived"),
        ],
        "status",
    );
    断言枚举(
        &[
            (FillingMode::Anonymous, "anonymous"),
            (FillingMode::LoginOnce, "login_once"),
        ],
        "mode",
    );
    断言枚举(
        &[(UserRole::User, "user"), (UserRole::Admin, "admin")],
        "role",
    );
    断言枚举(
        &[
            (AuditAction::DisableUser, "disable_user"),
            (AuditAction::EnableUser, "enable_user"),
            (AuditAction::TakedownSurvey, "take_down"),
            (AuditAction::RestoreSurvey, "restore_survey"),
        ],
        "action",
    );
}

#[tokio::test]
async fn 新提交资格按问卷状态给出具体错误() {
    let now = Utc::now();
    let 未来 = now + Duration::seconds(60);
    let 过去 = now - Duration::seconds(60);
    ensure_collecting(SurveyStatus::Collecting, None, false, now).expect("无限期回收可提交");
    ensure_collecting(SurveyStatus::Collecting, Some(未来), false, now).expect("未到期可提交");

    for (status, deadline, taken_down, 期望状态, 期望码, 期望消息) in [
        (
            SurveyStatus::Unpublished,
            None,
            true,
            StatusCode::CONFLICT,
            "taken_down",
            "问卷已被下架",
        ),
        (
            SurveyStatus::Archived,
            Some(过去),
            true,
            StatusCode::CONFLICT,
            "taken_down",
            "问卷已被下架",
        ),
        (
            SurveyStatus::Unpublished,
            None,
            false,
            StatusCode::NOT_FOUND,
            "not_found",
            "资源不存在或不可访问",
        ),
        (
            SurveyStatus::Collecting,
            Some(过去),
            false,
            StatusCode::CONFLICT,
            "deadline_passed",
            "问卷已到截止时间",
        ),
        (
            SurveyStatus::Collecting,
            Some(now),
            false,
            StatusCode::CONFLICT,
            "deadline_passed",
            "问卷已到截止时间",
        ),
        (
            SurveyStatus::Closed,
            None,
            false,
            StatusCode::CONFLICT,
            "collection_closed",
            "问卷已关闭回收",
        ),
        (
            SurveyStatus::Archived,
            None,
            false,
            StatusCode::CONFLICT,
            "archived",
            "问卷已归档",
        ),
    ] {
        let error = ensure_collecting(status, deadline, taken_down, now).expect_err("必须拒绝");
        let (状态, 码, 消息) = 三元组(error).await;
        assert_eq!(状态, 期望状态, "{status:?}");
        assert_eq!(码, 期望码, "{status:?}");
        assert_eq!(消息, 期望消息, "{status:?}");
    }
}

#[test]
fn 回收设置的截止时间区分省略清空与设定() {
    let 省略 = serde_json::from_str::<CollectionSettings>(r#"{"mode":"anonymous"}"#)
        .expect("省略 deadline 合法");
    assert_eq!(省略.mode, Some(FillingMode::Anonymous));
    assert_eq!(省略.deadline, None, "省略字段保留原值");

    let 清空 =
        serde_json::from_str::<CollectionSettings>(r#"{"mode":"anonymous","deadline":null}"#)
            .expect("显式 null 合法");
    assert_eq!(清空.deadline, Some(None), "显式 null 表示清空");

    let 设定 = serde_json::from_str::<CollectionSettings>(
        r#"{"mode":"login_once","deadline":"2026-12-31T12:00:00Z"}"#,
    )
    .expect("设定截止时间合法");
    assert_eq!(设定.mode, Some(FillingMode::LoginOnce));
    match 设定.deadline {
        Some(Some(截止)) => assert_eq!(截止.to_rfc3339(), "2026-12-31T12:00:00+00:00"),
        其他 => panic!("设定截止时间必须为 Some(Some(..))：{其他:?}"),
    }

    let error = serde_json::from_str::<CollectionSettings>(r#"{"mode":"匿名"}"#)
        .expect_err("未知填写模式必须拒绝");
    assert!(
        error.to_string().contains("unknown variant"),
        "未知填写模式必须定位到枚举变体：{error}"
    );
}

#[tokio::test]
async fn 归档问卷不能编辑或发布() {
    let mut record = survey_model();
    ensure_editable(&SurveyMetaRow::from(&record)).expect("未发布草稿可编辑");
    record.status = SurveyStatus::Collecting;
    ensure_editable(&SurveyMetaRow::from(&record)).expect("回收中可编辑");
    record.status = SurveyStatus::Closed;
    ensure_editable(&SurveyMetaRow::from(&record)).expect("已关闭仍可归档或复制");

    record.status = SurveyStatus::Archived;
    断言冲突(
        ensure_editable(&SurveyMetaRow::from(&record)).expect_err("归档必须拒绝"),
        "survey_archived",
        "已归档问卷不能编辑或发布，可以复制为新问卷",
    )
    .await;
}

#[tokio::test]
async fn 修订号必须与读取时一致() {
    let record = survey_model();
    let meta = SurveyMetaRow::from(&record);
    ensure_revision(&meta, 1).expect("修订号一致");
    for 冲突 in [0, 2, -1] {
        断言冲突(
            ensure_revision(&meta, 冲突).expect_err("必须拒绝"),
            "revision_conflict",
            "草稿已在其他窗口修改，请重新读取后核对内容",
        )
        .await;
    }
}

#[tokio::test]
async fn 恢复与首次发布要求截止时间未到() {
    let now = Utc::now();
    ensure_future_deadline(None, now).expect("没有截止时间合法");
    ensure_future_deadline(Some(now + Duration::seconds(1)), now).expect("未来时间合法");
    for 已到 in [now, now - Duration::seconds(1)] {
        断言冲突(
            ensure_future_deadline(Some(已到), now).expect_err("到期必须拒绝"),
            "deadline_expired",
            "截止时间已到，请先调整截止时间",
        )
        .await;
    }
}

#[test]
fn 管理详情只在发布后提供分享链接() {
    let now = Utc::now();
    let record = survey_model();
    let 编号 = record.id;
    let 文档: Document = serde_json::from_value(record.draft.clone()).expect("草稿必须是合法文档");

    let 草案 = survey_detail(SurveyMetaRow::from(&record), 文档.clone(), now);
    assert_eq!(草案.id, 编号);
    assert_eq!(草案.status, SurveyStatus::Unpublished);
    assert_eq!(草案.revision, 1);
    assert!(草案.current_version_id.is_none());
    assert!(草案.share_url.is_none(), "未发布不提供分享链接");
    assert_eq!(草案.draft.title, "测试问卷");

    let mut 已发布 = record.clone();
    已发布.status = SurveyStatus::Collecting;
    已发布.current_version_id = Some(Uuid::new_v4());
    已发布.mode = Some(FillingMode::Anonymous);
    let 详情 = survey_detail(SurveyMetaRow::from(&已发布), 文档.clone(), now);
    assert_eq!(
        详情.share_url,
        Some(format!("/api/public/surveys/{编号}")),
        "链接指向问卷而不是某个版本"
    );
    assert_eq!(详情.mode, Some(FillingMode::Anonymous));

    let mut 已到期 = 已发布;
    已到期.deadline = Some(now - Duration::seconds(1));
    assert_eq!(
        survey_detail(SurveyMetaRow::from(&已到期), 文档, now).status,
        SurveyStatus::Closed,
        "详情必须按读取时刻物化状态"
    );
}

#[tokio::test]
async fn 状态转移矩阵覆盖三个动作与四个状态() {
    let now = Utc::now();
    let 未来 = now + Duration::seconds(60);
    for (动作, action, current, 期望) in [
        (
            "关闭回收中",
            CollectionAction::Close,
            SurveyStatus::Collecting,
            SurveyStatus::Closed,
        ),
        (
            "重复关闭",
            CollectionAction::Close,
            SurveyStatus::Closed,
            SurveyStatus::Closed,
        ),
        (
            "归档回收中",
            CollectionAction::Archive,
            SurveyStatus::Collecting,
            SurveyStatus::Archived,
        ),
        (
            "归档已关闭",
            CollectionAction::Archive,
            SurveyStatus::Closed,
            SurveyStatus::Archived,
        ),
        (
            "重复归档",
            CollectionAction::Archive,
            SurveyStatus::Archived,
            SurveyStatus::Archived,
        ),
        (
            "恢复已关闭",
            CollectionAction::Resume,
            SurveyStatus::Closed,
            SurveyStatus::Collecting,
        ),
    ] {
        assert_eq!(
            next_collection_status(action, current, Some(未来), now).expect("合法转移"),
            期望,
            "{动作}"
        );
    }

    for (动作, action, current, 期望码) in [
        (
            "关闭未发布",
            CollectionAction::Close,
            SurveyStatus::Unpublished,
            "survey_not_published",
        ),
        (
            "关闭已归档",
            CollectionAction::Close,
            SurveyStatus::Archived,
            "survey_archived",
        ),
        (
            "恢复回收中",
            CollectionAction::Resume,
            SurveyStatus::Collecting,
            "survey_not_closed",
        ),
    ] {
        let error =
            next_collection_status(action, current, Some(未来), now).expect_err("非法转移必须拒绝");
        let (状态, 码, _) = 三元组(error).await;
        assert_eq!(状态, StatusCode::CONFLICT, "{动作}");
        assert_eq!(码, 期望码, "{动作}");
    }

    assert_eq!(
        next_collection_status(CollectionAction::Resume, SurveyStatus::Closed, None, now)
            .expect("没有截止时间可以恢复"),
        SurveyStatus::Collecting
    );
}

#[tokio::test]
async fn 填写模式在首次发布后冻结() {
    let 版本 = Uuid::new_v4();
    validate_mode_change(None, None, Some(FillingMode::Anonymous)).expect("未发布可设置模式");
    validate_mode_change(None, None, None).expect("可以省略模式");
    validate_mode_change(
        None,
        Some(FillingMode::Anonymous),
        Some(FillingMode::LoginOnce),
    )
    .expect("未发布可以更改模式");
    validate_mode_change(
        Some(版本),
        Some(FillingMode::Anonymous),
        Some(FillingMode::Anonymous),
    )
    .expect("重复原模式允许");
    validate_mode_change(Some(版本), Some(FillingMode::Anonymous), None)
        .expect("省略模式不改变已冻结的设置");

    for (current, requested) in [
        (Some(FillingMode::Anonymous), Some(FillingMode::LoginOnce)),
        (None, Some(FillingMode::Anonymous)),
    ] {
        断言冲突(
            validate_mode_change(Some(版本), current, requested).expect_err("已发布必须冻结"),
            "mode_frozen",
            "首次发布后不能更改填写模式",
        )
        .await;
    }
}

#[test]
fn 列表筛选接受标题状态与分页() {
    let 筛选: SurveyFilters =
        serde_json::from_str(r#"{"title":"甲","status":"closed","page":2,"page_size":50}"#)
            .expect("合法筛选必须解析");
    assert_eq!(筛选.title.as_deref(), Some("甲"));
    assert_eq!(筛选.status, Some(SurveyStatus::Closed));

    let 空筛选: SurveyFilters = serde_json::from_str("{}").expect("全部字段可省略");
    assert_eq!(空筛选.title, None);
    assert_eq!(空筛选.status, None);

    assert!(serde_json::from_str::<SurveyFilters>(r#"{"status":"deleted"}"#).is_err());
}

/// 构造指定版本与变体位的最小 UUID。
fn 带版本(version: u8, variant: u8) -> Uuid {
    let mut bytes = [0x3f_u8; 16];
    bytes[6] = (bytes[6] & 0x0f) | (version << 4);
    bytes[8] = (bytes[8] & 0x3f) | variant;
    Uuid::from_bytes(bytes)
}

#[tokio::test]
async fn 幂等秘密必须是安全随机_uuidv4() {
    validate_idempotency_key(Uuid::new_v4()).expect("UUIDv4 合法");
    for (说明, key) in [
        ("Nil", Uuid::nil()),
        ("版本 1", 带版本(1, 0x80)),
        ("版本 3", 带版本(3, 0x80)),
        ("版本 5", 带版本(5, 0x80)),
        ("Microsoft 变体", 带版本(4, 0xc0)),
    ] {
        let error = validate_idempotency_key(key).expect_err("必须拒绝");
        let (状态, 码, 消息) = 三元组(error).await;
        assert_eq!(状态, StatusCode::UNPROCESSABLE_ENTITY, "{说明}");
        assert_eq!(码, "validation_failed", "{说明}");
        assert_eq!(消息, "idempotency_key 必须是安全随机生成的 UUIDv4");
    }
}

#[test]
fn 载荷摘要只取决于版本与规范化答案() {
    let 版本 = Uuid::new_v4();
    let 题目 = Uuid::new_v4();
    let mut 答案 = BTreeMap::new();
    答案.insert(题目, json!("选项甲"));
    let 摘要 = submission_payload_hash(版本, &答案).expect("摘要成功");
    assert_eq!(摘要.len(), 64, "SHA-256 十六进制");
    assert_eq!(
        submission_payload_hash(版本, &答案).expect("摘要可重算"),
        摘要,
        "相同输入必须得到同一摘要"
    );
    assert_ne!(
        submission_payload_hash(Uuid::new_v4(), &答案).expect("摘要成功"),
        摘要,
        "版本是载荷的一部分"
    );

    let mut 其他答案 = 答案.clone();
    其他答案.insert(题目, json!("选项乙"));
    assert_ne!(
        submission_payload_hash(版本, &其他答案).expect("摘要成功"),
        摘要,
        "答案变化必须改变摘要"
    );
    assert_ne!(
        submission_payload_hash(版本, &BTreeMap::new()).expect("摘要成功"),
        摘要,
        "空答案与有答案是不同载荷"
    );
}

#[test]
fn 隐私提示按填写模式区分且不宣称完全匿名() {
    let 匿名 = privacy_notice(FillingMode::Anonymous);
    let 登录 = privacy_notice(FillingMode::LoginOnce);
    assert_ne!(匿名.mode_notice, 登录.mode_notice, "两种模式的提示必须不同");
    assert!(
        !匿名.mode_notice.contains("完全匿名"),
        "匿名模式不能宣称完全匿名"
    );
    assert!(
        登录.mode_notice.contains("不能称为完全匿名"),
        "登录模式必须明确否认完全匿名"
    );
}

#[tokio::test]
async fn 幂等冲突使用独立错误码() {
    let (状态, 码, _) = 三元组(idempotency_conflict()).await;
    assert_eq!(状态, StatusCode::CONFLICT);
    assert_eq!(码, "idempotency_conflict");
}
