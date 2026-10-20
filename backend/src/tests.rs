//! 跨模块单元测试：HTTP 边界解析、CSV 公式防护与问卷状态换算。

use crate::{
    domain::{AnswerState, Choice, Document, DocumentLogic, Question, QuestionKind},
    entities::{AuditAction, FillingMode, SurveyStatus, UserRole},
    http::{PageBounds, Pagination},
    reports::{
        QuestionStatisticsData, ReportFilter, SnapshotRow, TimeRange, encode_csv, formula_safe,
        summarize_question,
    },
    surveys::CollectionSettings,
};
use chrono::{Duration, Utc};
use sea_orm::ActiveEnum;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};
use uuid::Uuid;

fn pagination(page: Option<u64>, page_size: Option<u64>) -> Pagination {
    Pagination { page, page_size }
}

#[test]
fn 默认分页从第一页开始且每页二十条() {
    assert_eq!(
        pagination(None, None).bounds().expect("默认分页合法"),
        PageBounds {
            page: 1,
            page_size: 20,
            offset: 0,
            limit: 20,
        }
    );
}

#[test]
fn 分页参数按一与一百的边界换算偏移() {
    assert_eq!(
        pagination(Some(5), Some(100)).bounds().expect("上界合法"),
        PageBounds {
            page: 5,
            page_size: 100,
            offset: 400,
            limit: 100,
        }
    );
    assert_eq!(
        pagination(Some(1), Some(1)).bounds().expect("下界合法"),
        PageBounds {
            page: 1,
            page_size: 1,
            offset: 0,
            limit: 1,
        }
    );
}

#[test]
fn 非法页码与页大小被拦截() {
    assert!(pagination(Some(0), Some(20)).bounds().is_err());
    assert!(pagination(Some(1), Some(0)).bounds().is_err());
    assert!(pagination(Some(1), Some(101)).bounds().is_err());
    assert!(pagination(Some(u64::MAX), Some(100)).bounds().is_err());
}

#[test]
fn 普通文本直接借用不分配() {
    assert!(matches!(formula_safe("答卷标题"), Cow::Borrowed(_)));
    assert!(matches!(formula_safe(""), Cow::Borrowed(_)));
    assert!(matches!(formula_safe("a=b"), Cow::Borrowed(_)));
}

#[test]
fn 公式前缀与前导不可见字符被加单引号() {
    assert_eq!(formula_safe("=SUM(A1)"), "'=SUM(A1)");
    assert_eq!(formula_safe("+1"), "'+1");
    assert_eq!(formula_safe("-1"), "'-1");
    assert_eq!(formula_safe("@cmd"), "'@cmd");
    assert_eq!(formula_safe(" \t=1"), "' \t=1");
    assert_eq!(formula_safe("\u{feff}-2"), "'\u{feff}-2");
}

#[test]
fn 到期问卷在读取时视为已关闭() {
    let now = Utc::now();
    let past = now - Duration::seconds(1);
    let future = now + Duration::seconds(60);
    assert_eq!(
        SurveyStatus::Collecting.effective(Some(past), now),
        SurveyStatus::Closed
    );
    assert_eq!(
        SurveyStatus::Collecting.effective(Some(future), now),
        SurveyStatus::Collecting
    );
    assert_eq!(
        SurveyStatus::Collecting.effective(None, now),
        SurveyStatus::Collecting
    );
    assert_eq!(
        SurveyStatus::Closed.effective(Some(past), now),
        SurveyStatus::Closed
    );
    assert_eq!(
        SurveyStatus::Unpublished.effective(Some(past), now),
        SurveyStatus::Unpublished
    );
    assert_eq!(
        SurveyStatus::Archived.effective(Some(past), now),
        SurveyStatus::Archived
    );
}

fn 状态表(条目: &[(Uuid, AnswerState)]) -> BTreeMap<Uuid, AnswerState> {
    条目.iter().copied().collect()
}

fn 答案表(条目: &[(Uuid, Value)]) -> BTreeMap<Uuid, Value> {
    条目.iter().cloned().collect()
}

fn 样本行(
    receipt_id: Uuid,
    answers: BTreeMap<Uuid, Value>,
    question_states: BTreeMap<Uuid, AnswerState>,
) -> SnapshotRow {
    SnapshotRow {
        receipt_id,
        submitted_at: Utc::now(),
        answers,
        question_states: Some(question_states),
    }
}

#[test]
fn 实体枚举的数据库值与序列化值对齐约束() {
    fn 约束值集合(列名: &str) -> BTreeSet<String> {
        let 前缀 = format!("{列名} IN (");
        let 行 = include_str!("schema.sql")
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

    fn 断言枚举值<T: ActiveEnum<Value = String> + Serialize>(
        变体: &[(T, &str)], 列名: &str
    ) {
        let 期望: BTreeSet<String> = 变体.iter().map(|(_, 文本)| (*文本).to_owned()).collect();
        assert_eq!(
            约束值集合(列名),
            期望,
            "{列名} 的 CHECK 约束集合变化时必须同步枚举定义"
        );
        let mut 实际 = BTreeSet::new();
        for (值, 期望文本) in 变体 {
            let 数据库值 = 值.to_value();
            let 序列化值 = serde_json::to_value(值).expect("枚举必须可序列化");
            assert_eq!(
                Value::String(数据库值.clone()),
                序列化值,
                "SeaORM 与 serde 的字符串值必须一致"
            );
            assert_eq!(数据库值.as_str(), *期望文本);
            实际.insert(数据库值.clone());
        }
        assert_eq!(实际, 期望);
    }

    断言枚举值(
        &[
            (SurveyStatus::Unpublished, "unpublished"),
            (SurveyStatus::Collecting, "collecting"),
            (SurveyStatus::Closed, "closed"),
            (SurveyStatus::Archived, "archived"),
        ],
        "status",
    );
    断言枚举值(
        &[
            (FillingMode::Anonymous, "anonymous"),
            (FillingMode::LoginOnce, "login_once"),
        ],
        "mode",
    );
    断言枚举值(
        &[(UserRole::User, "user"), (UserRole::Admin, "admin")],
        "role",
    );
    断言枚举值(
        &[
            (AuditAction::DisableUser, "disable_user"),
            (AuditAction::EnableUser, "enable_user"),
            (AuditAction::TakedownSurvey, "take_down"),
            (AuditAction::RestoreSurvey, "restore_survey"),
        ],
        "action",
    );
}

fn 时间过滤(from: Option<&str>, to: Option<&str>) -> ReportFilter {
    ReportFilter {
        version_id: None,
        from: from.map(str::to_owned),
        to: to.map(str::to_owned),
    }
}

#[test]
fn 时间范围要求带时区且拒绝亚微秒非零截断与倒置区间() {
    let 六位微秒 = TimeRange::parse(&时间过滤(Some("2026-10-20T08:00:00.123456Z"), None))
        .expect("6 位微秒合法");
    let 全零纳秒 = TimeRange::parse(&时间过滤(
        Some("2026-10-20T16:00:00.123456000+08:00"),
        None,
    ))
    .expect("第 7 位起全零的纳秒表示合法");
    assert_eq!(
        六位微秒.from, 全零纳秒.from,
        "两种写法必须换算为同一 UTC 时刻"
    );

    assert!(
        TimeRange::parse(&时间过滤(Some("2026-10-20T08:00:00.1234567Z"), None)).is_err(),
        "第 7 位非零的亚微秒必须拒绝而不是截断"
    );
    assert!(
        TimeRange::parse(&时间过滤(Some("2026-10-20T08:00:00"), None)).is_err(),
        "缺少时区的时间必须拒绝"
    );
    for (from, to) in [
        ("2026-10-20T08:00:00Z", "2026-10-20T08:00:00Z"),
        ("2026-10-20T09:00:00Z", "2026-10-20T08:00:00Z"),
    ] {
        assert!(
            TimeRange::parse(&时间过滤(Some(from), Some(to))).is_err(),
            "开始时间必须严格早于结束时间：{from} -> {to}"
        );
    }
}

fn 统计用题目() -> (Question, Question, Question, Choice, Choice) {
    let 第一个选项 = Choice {
        id: Uuid::new_v4(),
        label: "甲".to_owned(),
    };
    let 第二个选项 = Choice {
        id: Uuid::new_v4(),
        label: "乙".to_owned(),
    };
    let 选择题 = Question {
        id: Uuid::new_v4(),
        title: "选择题".to_owned(),
        required: true,
        kind: QuestionKind::SingleChoice {
            options: vec![第一个选项.clone(), 第二个选项.clone()],
        },
    };
    let 数字题 = Question {
        id: Uuid::new_v4(),
        title: "数字题".to_owned(),
        required: false,
        kind: QuestionKind::Number {
            integer: false,
            min: None,
            max: None,
        },
    };
    let 评分题 = Question {
        id: Uuid::new_v4(),
        title: "评分题".to_owned(),
        required: false,
        kind: QuestionKind::Rating {
            min: 1,
            max: 5,
            min_label: String::new(),
            max_label: String::new(),
        },
    };
    (选择题, 数字题, 评分题, 第一个选项, 第二个选项)
}

#[test]
fn 统计聚合保持三态恒等式且导出包含规则状态列与公式转义() {
    let (选择题, 数字题, 评分题, 第一个选项, 第二个选项) = 统计用题目();
    let 已答 = Uuid::new_v4();
    let 适用未答 = Uuid::new_v4();
    let 规则跳过 = Uuid::new_v4();
    let 行 = vec![
        样本行(
            已答,
            答案表(&[
                (选择题.id, Value::String(第一个选项.id.to_string())),
                (数字题.id, json!(4.0)),
                (评分题.id, json!(5)),
            ]),
            状态表(&[
                (选择题.id, AnswerState::Answered),
                (数字题.id, AnswerState::Answered),
                (评分题.id, AnswerState::Answered),
            ]),
        ),
        样本行(
            适用未答,
            答案表(&[]),
            状态表(&[
                (选择题.id, AnswerState::Unanswered),
                (数字题.id, AnswerState::Unanswered),
                (评分题.id, AnswerState::Unanswered),
            ]),
        ),
        样本行(
            规则跳过,
            答案表(&[]),
            状态表(&[
                (选择题.id, AnswerState::Skipped),
                (数字题.id, AnswerState::Skipped),
                (评分题.id, AnswerState::Skipped),
            ]),
        ),
    ];
    let 统计 = vec![
        summarize_question(选择题.clone(), &行).expect("选择题统计成功"),
        summarize_question(数字题.clone(), &行).expect("数字题统计成功"),
        summarize_question(评分题.clone(), &行).expect("评分题统计成功"),
    ];
    for 一项 in &统计 {
        assert_eq!(一项.applicable_count, 2, "适用数等于已答加适用未答");
        assert_eq!(一项.answered_count, 1);
        assert_eq!(一项.unanswered_count, 1);
        assert_eq!(一项.skipped_count, 1);
        assert_eq!(
            一项.applicable_count + 一项.skipped_count,
            行.len() as u64,
            "适用数加规则跳过数必须等于样本总数"
        );
    }
    match &统计[0].statistics {
        QuestionStatisticsData::SingleChoice { options } => {
            assert_eq!(options[0].count, 1);
            assert_eq!(options[0].proportion, Some(1.0));
            assert_eq!(options[1].option_id, 第二个选项.id);
            assert_eq!(options[1].count, 0);
            assert_eq!(options[1].proportion, Some(0.0));
        }
        _ => panic!("选择题必须返回单选统计"),
    }
    match &统计[1].statistics {
        QuestionStatisticsData::Number { mean, min, max } => {
            assert_eq!(*mean, Some(4.0));
            assert_eq!(*min, Some(4.0));
            assert_eq!(*max, Some(4.0));
        }
        _ => panic!("数字题必须返回数字统计"),
    }
    match &统计[2].statistics {
        QuestionStatisticsData::Rating { mean, .. } => assert_eq!(*mean, Some(5.0)),
        _ => panic!("评分题必须返回评分统计"),
    }

    let 空样本: Vec<SnapshotRow> = Vec::new();
    for 题目 in [选择题, 数字题, 评分题] {
        let 零作答 = summarize_question(题目, &空样本).expect("零作答统计成功");
        assert_eq!(零作答.applicable_count, 0);
        assert_eq!(零作答.answered_count, 0);
        match 零作答.statistics {
            QuestionStatisticsData::SingleChoice { options } => {
                assert!(options.iter().all(|选项| 选项.proportion.is_none()));
            }
            QuestionStatisticsData::Number { mean, min, max } => {
                assert_eq!(mean, None);
                assert_eq!(min, None);
                assert_eq!(max, None);
            }
            QuestionStatisticsData::Rating { mean, .. } => assert_eq!(mean, None),
            _ => panic!("未覆盖的题型"),
        }
    }

    let 文本题 = Question {
        id: Uuid::new_v4(),
        title: "文本题".to_owned(),
        required: false,
        kind: QuestionKind::Text { max_length: 200 },
    };
    let 导出文档 = Document {
        logic: Some(DocumentLogic {
            flow: None,
            dsl_version: 1u32.into(),
            semantics_version: 1u32.into(),
            variables: Vec::new(),
            rules: Vec::new(),
        }),
        title: "导出测试".to_owned(),
        description: String::new(),
        purpose: "测试".to_owned(),
        thank_you: "谢谢".to_owned(),
        questions: vec![文本题.clone()],
    };
    let 导出样本 = vec![
        样本行(
            Uuid::new_v4(),
            答案表(&[(文本题.id, Value::String("=1+1".to_owned()))]),
            状态表(&[(文本题.id, AnswerState::Answered)]),
        ),
        样本行(
            Uuid::new_v4(),
            答案表(&[]),
            状态表(&[(文本题.id, AnswerState::Unanswered)]),
        ),
        样本行(
            Uuid::new_v4(),
            答案表(&[]),
            状态表(&[(文本题.id, AnswerState::Skipped)]),
        ),
    ];
    let 字节 = encode_csv(&导出文档, Uuid::new_v4(), 3, &导出样本).expect("CSV 导出成功");
    let mut 读取器 = csv::Reader::from_reader(字节.as_slice());
    let 表头 = 读取器.headers().expect("CSV 表头可解析").clone();
    let 状态表头 = format!("状态 [{}]", 文本题.id);
    let 题目表头 = format!("{} [{}]", 文本题.title, 文本题.id);
    let 状态列 = 表头
        .iter()
        .position(|字段| 字段 == 状态表头.as_str())
        .expect("带规则的文档必须包含状态列");
    let 题目列 = 表头
        .iter()
        .position(|字段| 字段 == 题目表头.as_str())
        .expect("表头必须包含题目 ID");
    let 记录: Vec<csv::StringRecord> = 读取器
        .records()
        .map(|记录| 记录.expect("CSV 行可解析"))
        .collect();
    assert_eq!(记录.len(), 3);
    assert_eq!(
        记录[0].get(题目列),
        Some("'=1+1"),
        "公式前缀必须被单引号转义"
    );
    assert_eq!(记录[0].get(状态列), Some("已答"));
    assert_eq!(记录[1].get(状态列), Some("适用未答"));
    assert_eq!(记录[2].get(状态列), Some("规则跳过"));
}

#[test]
fn 回收设置截止时间区分省略清空与设定三态且分页兼容字符串数字() {
    let 省略 = serde_json::from_str::<CollectionSettings>(r#"{"mode": "anonymous"}"#)
        .expect("省略 deadline 合法");
    assert_eq!(省略.mode, Some(FillingMode::Anonymous));
    assert_eq!(省略.deadline, None, "省略字段保留原值");

    let 清空 =
        serde_json::from_str::<CollectionSettings>(r#"{"mode": "anonymous", "deadline": null}"#)
            .expect("显式 null 合法");
    assert_eq!(清空.deadline, Some(None), "显式 null 表示清空");

    let 设定 = serde_json::from_str::<CollectionSettings>(
        r#"{"mode": "login_once", "deadline": "2026-12-31T12:00:00Z"}"#,
    )
    .expect("设定截止时间合法");
    assert_eq!(设定.mode, Some(FillingMode::LoginOnce));
    match 设定.deadline {
        Some(Some(截止)) => assert_eq!(截止.to_rfc3339(), "2026-12-31T12:00:00+00:00"),
        其他 => panic!("设定截止时间必须为 Some(Some(..))：{其他:?}"),
    }

    let 字符串数字 = serde_json::from_str::<Pagination>(r#"{"page": "2", "page_size": "50"}"#)
        .expect("字符串数字兼容");
    let 数字 =
        serde_json::from_str::<Pagination>(r#"{"page": 2, "page_size": 50}"#).expect("数字兼容");
    assert_eq!(字符串数字.page, Some(2));
    assert_eq!(字符串数字.page_size, Some(50));
    let 期望 = PageBounds {
        page: 2,
        page_size: 50,
        offset: 50,
        limit: 50,
    };
    assert_eq!(字符串数字.bounds().expect("字符串数字分页合法"), 期望);
    assert_eq!(数字.bounds().expect("数字分页合法"), 期望);
}
