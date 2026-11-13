//! 报表边界：时间范围、版本选择、持久化三态、统计聚合与分批 CSV 注入防护。

use std::collections::BTreeMap;

use axum::http::StatusCode;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    domain::{AnswerState, Choice, Document, DocumentLogic, Question, QuestionKind},
    entities::Outcome,
    reports::{
        QuestionStatisticsData, ReportFilter, SnapshotRow, TimeRange, encode_csv,
        encode_csv_header, encode_csv_rows, formula_safe, resolve_version_id, summarize_question,
        summarize_report, update_mean,
    },
    tests::support::error_json,
};

fn 时刻(文本: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(文本)
        .expect("测试时间必须是合法 RFC3339")
        .with_timezone(&Utc)
}

fn 过滤(from: Option<&str>, to: Option<&str>) -> ReportFilter {
    ReportFilter {
        version_id: None,
        from: from.map(str::to_owned),
        to: to.map(str::to_owned),
    }
}

async fn 时间错误(from: Option<&str>, to: Option<&str>) -> String {
    let error = TimeRange::parse(&过滤(from, to))
        .err()
        .expect("非法时间必须拒绝");
    let (状态, _, body) = error_json(error).await;
    assert_eq!(状态, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "validation_failed");
    body["error"]["message"]
        .as_str()
        .expect("错误消息必须是字符串")
        .to_owned()
}

#[test]
fn 时间范围省略任一端都合法() {
    let 仅起 = TimeRange::parse(&过滤(Some("2026-10-20T08:00:00Z"), None)).expect("只有开始时间");
    assert_eq!(仅起.from, Some(时刻("2026-10-20T08:00:00Z")));
    assert_eq!(仅起.to, None);

    let 仅止 = TimeRange::parse(&过滤(None, Some("2026-10-20T08:00:00Z"))).expect("只有结束时间");
    assert_eq!(仅止.from, None);
    assert_eq!(仅止.to, Some(时刻("2026-10-20T08:00:00Z")));

    let 双端 = TimeRange::parse(&过滤(
        Some("2026-10-20T08:00:00Z"),
        Some("2026-10-21T08:00:00Z"),
    ))
    .expect("双端合法");
    assert_eq!(双端.from, Some(时刻("2026-10-20T08:00:00Z")));
    assert_eq!(双端.to, Some(时刻("2026-10-21T08:00:00Z")));

    let 空 = TimeRange::parse(&过滤(None, None)).expect("都不传也合法");
    assert_eq!(空.from, None);
    assert_eq!(空.to, None);
}

#[tokio::test]
async fn 时间必须携带时区且由标准解析器换算() {
    let 微秒 =
        TimeRange::parse(&过滤(Some("2026-10-20T08:00:00.123456Z"), None)).expect("6 位小数合法");
    let 别区写法 = TimeRange::parse(&过滤(Some("2026-10-20T16:00:00.123456+08:00"), None))
        .expect("带时区偏移的 RFC3339 合法");
    assert_eq!(微秒.from, 别区写法.from, "不同时区写法必须换算为同一时刻");
    // 标准解析器负责精度；超出微秒的输入不再手工切片校验。
    TimeRange::parse(&过滤(Some("2026-10-20T08:00:00.1234567Z"), None))
        .expect("7 位小数由 chrono 解析");
    TimeRange::parse(&过滤(Some("2026-10-20T08:00:00.123456789Z"), None))
        .expect("9 位小数由 chrono 解析");

    assert_eq!(
        时间错误(Some("2026-10-20T08:00:00"), None).await,
        "时间必须是包含时区的 RFC3339 格式"
    );
    assert_eq!(
        时间错误(Some("不是时间"), None).await,
        "时间必须是包含时区的 RFC3339 格式"
    );
}

#[tokio::test]
async fn 开始时间必须严格早于结束时间() {
    for (from, to) in [
        ("2026-10-20T08:00:00Z", "2026-10-20T08:00:00Z"),
        ("2026-10-20T09:00:00Z", "2026-10-20T08:00:00Z"),
    ] {
        assert_eq!(
            时间错误(Some(from), Some(to)).await,
            "开始时间必须早于结束时间",
            "{from} -> {to}"
        );
    }
}

#[test]
fn 报表选择把时间换算为展示时区() {
    let range = TimeRange::parse(&过滤(
        Some("2026-10-20T00:00:00Z"),
        Some("2026-10-21T00:00:00Z"),
    ))
    .expect("合法时间范围");
    let 版本 = Uuid::new_v4();
    let selection = range.selection(版本, 7);
    assert_eq!(selection.version_id, 版本);
    assert_eq!(selection.version_number, 7);
    assert_eq!(selection.timezone, "Asia/Shanghai (+08:00)");
    let from = selection.from.expect("有开始时间");
    let to = selection.to.expect("有结束时间");
    assert_eq!(from.to_rfc3339(), "2026-10-20T08:00:00+08:00");
    assert_eq!(to.to_rfc3339(), "2026-10-21T08:00:00+08:00");
    assert_eq!(from.offset().local_minus_utc(), 8 * 3_600);
}

#[tokio::test]
async fn 版本解析优先显式参数再回退当前版本() {
    let 当前 = Uuid::new_v4();
    let 显式 = Uuid::new_v4();
    assert_eq!(
        resolve_version_id(Some(当前), Some(显式)).expect("显式版本优先"),
        显式
    );
    assert_eq!(
        resolve_version_id(Some(当前), None).expect("回退当前版本"),
        当前
    );

    let error = resolve_version_id(None, None).expect_err("未发布问卷没有可读版本");
    let (状态, _, body) = error_json(error).await;
    assert_eq!(状态, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "not_published");
    assert_eq!(
        body["error"]["message"],
        "问卷尚未发布，没有可查看的答卷版本"
    );
}

fn 答案(条目: &[(Uuid, Value)]) -> BTreeMap<Uuid, Value> {
    条目.iter().cloned().collect()
}

fn 状态(条目: &[(Uuid, AnswerState)]) -> BTreeMap<Uuid, AnswerState> {
    条目.iter().copied().collect()
}

fn 完成行(
    receipt_id: Uuid,
    submitted_at: DateTime<Utc>,
    answers: BTreeMap<Uuid, Value>,
    question_states: BTreeMap<Uuid, AnswerState>,
) -> SnapshotRow {
    SnapshotRow {
        receipt_id,
        submitted_at,
        answers,
        question_states,
        outcome: Outcome::Complete,
    }
}

fn 筛选结束行(
    receipt_id: Uuid,
    submitted_at: DateTime<Utc>,
    answers: BTreeMap<Uuid, Value>,
    question_states: BTreeMap<Uuid, AnswerState>,
) -> SnapshotRow {
    SnapshotRow {
        receipt_id,
        submitted_at,
        answers,
        question_states,
        outcome: Outcome::ScreenOut,
    }
}

fn 题目(id: Uuid, title: &str, kind: QuestionKind) -> Question {
    Question {
        id,
        title: title.to_owned(),
        required: false,
        kind,
    }
}

fn 单选题目() -> (Question, Choice, Choice) {
    let 甲 = Choice {
        id: Uuid::new_v4(),
        label: "甲".to_owned(),
    };
    let 乙 = Choice {
        id: Uuid::new_v4(),
        label: "乙".to_owned(),
    };
    let 题 = 题目(
        Uuid::new_v4(),
        "单选题",
        QuestionKind::SingleChoice {
            options: vec![甲.clone(), 乙.clone()],
        },
    );
    (题, 甲, 乙)
}

fn 单题文档(题: Question) -> Document {
    Document {
        logic: None,
        title: "统计问卷".to_owned(),
        description: String::new(),
        purpose: "测试用途".to_owned(),
        thank_you: "谢谢".to_owned(),
        questions: vec![题],
    }
}

#[test]
fn 统计区分完成与筛选结束且题目只统计完成样本() {
    let (题, 甲, _) = 单选题目();
    let 文档 = 单题文档(题.clone());
    let 时间 = 时刻("2026-10-20T00:00:00Z");
    let rows = vec![
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, json!(甲.id))]),
            状态(&[(题.id, AnswerState::Answered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[]),
            状态(&[(题.id, AnswerState::Unanswered)]),
        ),
        筛选结束行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, json!(甲.id))]),
            状态(&[(题.id, AnswerState::Answered)]),
        ),
    ];
    let 统计 = summarize_report(&文档, &rows).expect("统计成功");
    assert_eq!(统计.total_count, 3);
    assert_eq!(统计.complete_count, 2);
    assert_eq!(统计.screen_out_count, 1);
    assert_eq!(
        统计.total_count,
        统计.complete_count + 统计.screen_out_count
    );
    assert_eq!(统计.trend.len(), 1);
    assert_eq!(统计.trend[0].count, 3, "趋势包含筛选结束样本");
    assert_eq!(
        统计.trend[0].date,
        NaiveDate::from_ymd_opt(2026, 10, 20).expect("日期合法")
    );
    let 题统计 = &统计.questions[0];
    assert_eq!(题统计.applicable_count, 2, "筛选结束不进入题目分母");
    assert_eq!(题统计.answered_count, 1);
    assert_eq!(题统计.unanswered_count, 1);
    assert_eq!(题统计.skipped_count, 0);
}

#[test]
fn 单选统计按选项计数并以外答数为分母() {
    let (题, 甲, 乙) = 单选题目();
    let 时间 = 时刻("2026-10-20T00:00:00Z");
    let rows = vec![
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, json!(甲.id))]),
            状态(&[(题.id, AnswerState::Answered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, json!(乙.id))]),
            状态(&[(题.id, AnswerState::Answered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[]),
            状态(&[(题.id, AnswerState::Unanswered)]),
        ),
    ];
    let 统计 = summarize_question(&题, &rows).expect("统计成功");
    assert_eq!(统计.question_id, 题.id);
    assert_eq!(统计.title, "单选题");
    assert_eq!(统计.answered_count, 2);
    assert_eq!(统计.applicable_count, 3);
    assert_eq!(统计.unanswered_count, 1);
    assert_eq!(统计.skipped_count, 0);
    match 统计.statistics {
        QuestionStatisticsData::SingleChoice { options } => {
            assert_eq!(options.len(), 2);
            assert_eq!(options[0].option_id, 甲.id);
            assert_eq!(options[0].label, "甲");
            assert_eq!(options[0].count, 1);
            assert_eq!(options[0].proportion, Some(0.5));
            assert_eq!(options[1].option_id, 乙.id);
            assert_eq!(options[1].count, 1);
            assert_eq!(options[1].proportion, Some(0.5));
        }
        _ => panic!("单选题必须返回单选统计"),
    }
}

#[test]
fn 多选占比合计可以超过百分之百() {
    let 甲 = Choice {
        id: Uuid::new_v4(),
        label: "甲".to_owned(),
    };
    let 乙 = Choice {
        id: Uuid::new_v4(),
        label: "乙".to_owned(),
    };
    let 题 = 题目(
        Uuid::new_v4(),
        "多选题",
        QuestionKind::MultipleChoice {
            options: vec![甲.clone(), 乙.clone()],
            min_choices: 1,
            max_choices: 2,
        },
    );
    let 时间 = 时刻("2026-10-20T00:00:00Z");
    let rows = vec![
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, json!([甲.id, 乙.id]))]),
            状态(&[(题.id, AnswerState::Answered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, json!([甲.id]))]),
            状态(&[(题.id, AnswerState::Answered)]),
        ),
    ];
    let 统计 = summarize_question(&题, &rows).expect("统计成功");
    assert_eq!(统计.answered_count, 2);
    match 统计.statistics {
        QuestionStatisticsData::MultipleChoice { options, .. } => {
            assert_eq!(options[0].count, 2);
            assert_eq!(options[0].proportion, Some(1.0));
            assert_eq!(options[1].count, 1);
            assert_eq!(options[1].proportion, Some(0.5));
            let 合计: f64 = options
                .iter()
                .map(|选项| 选项.proportion.expect("有回答时占比非空"))
                .sum();
            assert!(合计 > 1.0, "多选占比合计可以超过 100%：{合计}");
        }
        _ => panic!("多选题必须返回多选统计"),
    }
}

#[test]
fn 文本题统计只返回计数不再内联正文() {
    let 单行 = 题目(
        Uuid::new_v4(),
        "单行文本",
        QuestionKind::Text { max_length: 200 },
    );
    let 时间 = 时刻("2026-10-20T00:00:00Z");
    let rows = vec![
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(单行.id, json!("第一条"))]),
            状态(&[(单行.id, AnswerState::Answered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[]),
            状态(&[(单行.id, AnswerState::Unanswered)]),
        ),
    ];
    let 统计 = summarize_question(&单行, &rows).expect("统计成功");
    assert_eq!(统计.answered_count, 1, "文本正文不回流到统计");
    assert_eq!(统计.applicable_count, 2);
    assert_eq!(统计.unanswered_count, 1);
    assert!(matches!(统计.statistics, QuestionStatisticsData::Text {}));
    let 序列化 = serde_json::to_value(&统计).expect("统计可序列化");
    assert_eq!(序列化["type"], "text");
    assert!(
        序列化.get("answers").is_none(),
        "文本统计不能再返回 answers 数组"
    );

    let 多行 = 题目(
        Uuid::new_v4(),
        "多行文本",
        QuestionKind::Textarea { max_length: 1_000 },
    );
    let 多行记录 = vec![完成行(
        Uuid::new_v4(),
        时间,
        答案(&[(多行.id, json!("第一行\n第二行"))]),
        状态(&[(多行.id, AnswerState::Answered)]),
    )];
    let 统计 = summarize_question(&多行, &多行记录).expect("统计成功");
    assert!(matches!(
        统计.statistics,
        QuestionStatisticsData::Textarea {}
    ));
    assert_eq!(统计.answered_count, 1);
}

#[test]
fn 数字统计排除缺失但把零当作有效值() {
    let 题 = 题目(
        Uuid::new_v4(),
        "数字题",
        QuestionKind::Number {
            integer: false,
            min: None,
            max: None,
        },
    );
    let 时间 = 时刻("2026-10-20T00:00:00Z");
    let rows = vec![
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, json!(0.0))]),
            状态(&[(题.id, AnswerState::Answered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, json!(4.0))]),
            状态(&[(题.id, AnswerState::Answered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[]),
            状态(&[(题.id, AnswerState::Unanswered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[]),
            状态(&[(题.id, AnswerState::Skipped)]),
        ),
    ];
    let 统计 = summarize_question(&题, &rows).expect("统计成功");
    assert_eq!(统计.answered_count, 2, "缺失值不计入回答数");
    assert_eq!(统计.applicable_count, 3);
    assert_eq!(统计.unanswered_count, 1);
    assert_eq!(统计.skipped_count, 1);
    match 统计.statistics {
        QuestionStatisticsData::Number { mean, min, max } => {
            assert_eq!(mean, Some(2.0), "0 是有效值，参与均值");
            assert_eq!(min, Some(0.0));
            assert_eq!(max, Some(4.0));
        }
        _ => panic!("数字题必须返回数字统计"),
    }
}

#[test]
fn 评分统计按分值升序给出分布与均值() {
    let 题 = 题目(
        Uuid::new_v4(),
        "评分题",
        QuestionKind::Rating {
            min: 1,
            max: 5,
            min_label: "很差".to_owned(),
            max_label: "很好".to_owned(),
        },
    );
    let 时间 = 时刻("2026-10-20T00:00:00Z");
    let rows = vec![
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, json!(5))]),
            状态(&[(题.id, AnswerState::Answered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, json!(3.0))]),
            状态(&[(题.id, AnswerState::Answered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[]),
            状态(&[(题.id, AnswerState::Unanswered)]),
        ),
    ];
    let 统计 = summarize_question(&题, &rows).expect("统计成功");
    assert_eq!(统计.answered_count, 2);
    assert_eq!(统计.applicable_count, 3);
    assert_eq!(统计.unanswered_count, 1);
    match 统计.statistics {
        QuestionStatisticsData::Rating {
            min,
            max,
            distribution,
            mean,
        } => {
            assert_eq!((min, max), (1, 5));
            assert_eq!(
                distribution
                    .iter()
                    .map(|项| (项.value, 项.count))
                    .collect::<Vec<_>>(),
                vec![(3, 1), (5, 1)],
                "分布必须按分值升序"
            );
            assert_eq!(mean, Some(4.0));
        }
        _ => panic!("评分题必须返回评分统计"),
    }
}

#[test]
fn 空样本集返回零计数与空统计() {
    let (单选, ..) = 单选题目();
    let 多选 = 题目(
        Uuid::new_v4(),
        "多选",
        QuestionKind::MultipleChoice {
            options: vec![
                Choice {
                    id: Uuid::new_v4(),
                    label: "甲".to_owned(),
                },
                Choice {
                    id: Uuid::new_v4(),
                    label: "乙".to_owned(),
                },
            ],
            min_choices: 1,
            max_choices: 2,
        },
    );
    let 数字 = 题目(
        Uuid::new_v4(),
        "数字",
        QuestionKind::Number {
            integer: false,
            min: None,
            max: None,
        },
    );
    let 评分 = 题目(
        Uuid::new_v4(),
        "评分",
        QuestionKind::Rating {
            min: 1,
            max: 5,
            min_label: String::new(),
            max_label: String::new(),
        },
    );
    let 文本 = 题目(
        Uuid::new_v4(),
        "文本",
        QuestionKind::Text { max_length: 200 },
    );
    let 空样本: Vec<SnapshotRow> = Vec::new();
    for 题 in [单选, 多选, 数字, 评分, 文本] {
        let 统计 = summarize_question(&题, &空样本).expect("空样本统计成功");
        assert_eq!(统计.applicable_count, 0);
        assert_eq!(统计.answered_count, 0);
        assert_eq!(统计.unanswered_count, 0);
        assert_eq!(统计.skipped_count, 0);
        match 统计.statistics {
            QuestionStatisticsData::SingleChoice { options }
            | QuestionStatisticsData::MultipleChoice { options, .. } => {
                assert!(options.iter().all(|选项| 选项.proportion.is_none()));
                assert!(options.iter().all(|选项| 选项.count == 0));
            }
            QuestionStatisticsData::Number { mean, min, max } => {
                assert_eq!((mean, min, max), (None, None, None));
            }
            QuestionStatisticsData::Rating {
                distribution, mean, ..
            } => {
                assert!(distribution.is_empty());
                assert_eq!(mean, None);
            }
            QuestionStatisticsData::Text {} | QuestionStatisticsData::Textarea {} => {}
        }
    }
}

#[test]
fn 三态计数满足适用加跳过等于完成样本数() {
    let (题, 甲, _) = 单选题目();
    let 时间 = 时刻("2026-10-20T00:00:00Z");
    let rows = vec![
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, json!(甲.id))]),
            状态(&[(题.id, AnswerState::Answered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[]),
            状态(&[(题.id, AnswerState::Unanswered)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[]),
            状态(&[(题.id, AnswerState::Skipped)]),
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[]),
            状态(&[(题.id, AnswerState::Skipped)]),
        ),
    ];
    let 统计 = summarize_question(&题, &rows).expect("统计成功");
    assert_eq!(统计.answered_count, 1);
    assert_eq!(统计.skipped_count, 2);
    assert_eq!(统计.applicable_count, 2);
    assert_eq!(统计.unanswered_count, 1);
    assert_eq!(
        统计.applicable_count + 统计.skipped_count,
        rows.len() as u64,
        "适用数加跳过数必须等于完成样本数"
    );
    assert_eq!(
        统计.unanswered_count,
        统计.applicable_count - 统计.answered_count
    );
}

#[tokio::test]
async fn 损坏的已存答案返回内部错误且不回显内容() {
    let (单选, ..) = 单选题目();
    let 数字 = 题目(
        Uuid::new_v4(),
        "数字",
        QuestionKind::Number {
            integer: false,
            min: None,
            max: None,
        },
    );
    let 时间 = 时刻("2026-10-20T00:00:00Z");
    for (题, 值) in [(单选, json!(42)), (数字, json!("秘密数值"))] {
        let rows = vec![完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(题.id, 值.clone())]),
            状态(&[(题.id, AnswerState::Answered)]),
        )];
        let error = summarize_question(&题, &rows)
            .err()
            .expect("题型与已存答案不符必须报错");
        let (状态, _, body) = error_json(error).await;
        assert_eq!(状态, StatusCode::INTERNAL_SERVER_ERROR);
        let 响应文本 = body.to_string();
        assert!(
            !响应文本.contains("秘密数值"),
            "错误响应不能回显已存答案：{响应文本}"
        );
    }
}

#[test]
fn 公式前缀与前导不可见字符被加上单引号() {
    assert_eq!(
        formula_safe("普通文本").as_ref(),
        "普通文本",
        "普通文本原样返回"
    );
    for 原文 in [
        "=SUM(A1)",
        "+1",
        "-1",
        "@cmd",
        " \t=1",
        "\u{feff}-2",
        "\u{200b}@cmd",
        "\u{0007}=1",
        "\u{202e}+1",
        "\u{2060}-1",
        "\u{00ad}@1",
    ] {
        let 安全 = formula_safe(原文);
        assert_eq!(
            安全.as_ref(),
            format!("'{原文}").as_str(),
            "只加单引号，不删除原文"
        );
    }
}

fn 导出文档(logic: Option<DocumentLogic>) -> (Document, Question, Question, Choice, Choice) {
    let 甲 = Choice {
        id: Uuid::new_v4(),
        label: "甲".to_owned(),
    };
    let 乙 = Choice {
        id: Uuid::new_v4(),
        label: "乙".to_owned(),
    };
    let 多选 = 题目(
        Uuid::new_v4(),
        "多选",
        QuestionKind::MultipleChoice {
            options: vec![甲.clone(), 乙.clone()],
            min_choices: 1,
            max_choices: 2,
        },
    );
    let 文本 = 题目(
        Uuid::new_v4(),
        "=SUM(A1)",
        QuestionKind::Text { max_length: 200 },
    );
    let 文档 = Document {
        logic,
        title: "导出问卷".to_owned(),
        description: String::new(),
        purpose: "测试".to_owned(),
        thank_you: "谢谢".to_owned(),
        questions: vec![多选.clone(), 文本.clone()],
    };
    (文档, 多选, 文本, 甲, 乙)
}

#[test]
fn 无逻辑版本的_csv_覆盖全部六种题型且防护表头公式与空值留空() {
    let (mut 文档, 多选, 文本, 甲, 乙) = 导出文档(None);
    let 单甲 = Choice {
        id: Uuid::new_v4(),
        label: "甲".to_owned(),
    };
    let 单乙 = Choice {
        id: Uuid::new_v4(),
        label: "乙".to_owned(),
    };
    let 单选 = 题目(
        Uuid::new_v4(),
        "单选",
        QuestionKind::SingleChoice {
            options: vec![单甲.clone(), 单乙],
        },
    );
    let 多行 = 题目(
        Uuid::new_v4(),
        "多行",
        QuestionKind::Textarea { max_length: 1_000 },
    );
    let 数字 = 题目(
        Uuid::new_v4(),
        "数字",
        QuestionKind::Number {
            integer: false,
            min: None,
            max: None,
        },
    );
    let 评分 = 题目(
        Uuid::new_v4(),
        "评分",
        QuestionKind::Rating {
            min: 1,
            max: 5,
            min_label: String::new(),
            max_label: String::new(),
        },
    );
    文档
        .questions
        .extend([单选.clone(), 多行.clone(), 数字.clone(), 评分.clone()]);

    let 时间 = 时刻("2026-10-20T00:00:00Z");
    let 版本 = Uuid::new_v4();
    let 多选值 = json!([甲.id.to_string(), 乙.id.to_string()]);
    let states: BTreeMap<Uuid, AnswerState> = 文档
        .questions
        .iter()
        .map(|question| (question.id, AnswerState::Answered))
        .collect();
    let rows = vec![
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[
                (多选.id, 多选值.clone()),
                (文本.id, json!("普通文本")),
                (单选.id, json!(单甲.id)),
                (多行.id, json!("第一行\n第二行")),
                (数字.id, json!(4.5)),
                (评分.id, json!(5)),
            ]),
            states,
        ),
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[]),
            状态(&[
                (多选.id, AnswerState::Unanswered),
                (文本.id, AnswerState::Unanswered),
                (单选.id, AnswerState::Unanswered),
                (多行.id, AnswerState::Unanswered),
                (数字.id, AnswerState::Unanswered),
                (评分.id, AnswerState::Unanswered),
            ]),
        ),
    ];
    let 字节 = encode_csv(&文档, 版本, 3, &rows).expect("导出成功");
    assert!(
        字节.windows(2).any(|窗口| 窗口 == b"\r\n".as_slice()),
        "必须使用 CRLF 换行"
    );

    let mut reader = csv::Reader::from_reader(字节.as_slice());
    let 表头 = reader.headers().expect("表头可解析").clone();
    assert_eq!(
        表头.iter().take(5).collect::<Vec<_>>(),
        vec![
            "receipt_id",
            "outcome",
            "version_id",
            "version_number",
            "submitted_at (Asia/Shanghai +08:00)",
        ]
    );
    assert_eq!(表头.len(), 11, "六道题且无状态列，另加 outcome");
    assert_eq!(&表头[5], format!("多选 [{}]", 多选.id).as_str());
    assert_eq!(
        &表头[6],
        format!("'=SUM(A1) [{}]", 文本.id).as_str(),
        "题目标题表头含公式前缀时同样转义"
    );
    assert_eq!(&表头[7], format!("单选 [{}]", 单选.id).as_str());
    assert_eq!(&表头[10], format!("评分 [{}]", 评分.id).as_str());

    let 记录: Vec<csv::StringRecord> = reader
        .records()
        .map(|记录| 记录.expect("CSV 行可解析"))
        .collect();
    assert_eq!(记录.len(), 2);
    assert_eq!(&记录[0][1], "complete", "每行带流程终点");
    assert_eq!(&记录[0][2], 版本.to_string().as_str());
    assert_eq!(&记录[0][3], "3");
    assert_eq!(&记录[0][4], "2026-10-20T08:00:00+08:00");
    assert_eq!(
        &记录[0][5],
        serde_json::to_string(&多选值)
            .expect("数组可序列化")
            .as_str(),
        "多选保持 JSON 数组"
    );
    assert_eq!(&记录[0][6], "普通文本");
    assert_eq!(
        &记录[0][7],
        单甲.id.to_string().as_str(),
        "单选导出选项 UUID 字符串"
    );
    assert_eq!(&记录[0][8], "第一行\n第二行", "多行文本保留原文与换行");
    assert_eq!(&记录[0][9], "4.5", "数字题导出 JSON 数字文本");
    assert_eq!(&记录[0][10], "5", "评分题导出整数文本");
    for 单元格 in 记录[1].iter().skip(5) {
        assert_eq!(单元格, "", "缺失答案留空单元格");
    }
}

#[test]
fn 有逻辑版本的_csv_带状态列且多选保持_json_数组() {
    let 逻辑 = DocumentLogic {
        flow: None,
        dsl_version: 1u32.into(),
        semantics_version: 1u32.into(),
        variables: Vec::new(),
        rules: Vec::new(),
    };
    let (文档, 多选, 文本, 甲, 乙) = 导出文档(Some(逻辑));
    let 时间 = 时刻("2026-10-20T00:00:00Z");
    let 多选值 = json!([甲.id.to_string(), 乙.id.to_string()]);
    let rows = vec![
        完成行(
            Uuid::new_v4(),
            时间,
            答案(&[(多选.id, 多选值.clone()), (文本.id, json!("=cmd"))]),
            状态(&[
                (多选.id, AnswerState::Answered),
                (文本.id, AnswerState::Answered),
            ]),
        ),
        筛选结束行(
            Uuid::new_v4(),
            时间,
            答案(&[]),
            状态(&[
                (多选.id, AnswerState::Unanswered),
                (文本.id, AnswerState::Skipped),
            ]),
        ),
    ];
    let 字节 = encode_csv(&文档, Uuid::new_v4(), 1, &rows).expect("导出成功");
    let mut reader = csv::Reader::from_reader(字节.as_slice());
    let 表头 = reader.headers().expect("表头可解析").clone();
    assert_eq!(表头.len(), 9, "两道题各带一个状态列，另加 outcome");
    assert_eq!(&表头[5], format!("多选 [{}]", 多选.id).as_str());
    assert_eq!(&表头[6], format!("状态 [{}]", 多选.id).as_str());
    assert_eq!(&表头[7], format!("'=SUM(A1) [{}]", 文本.id).as_str());
    assert_eq!(&表头[8], format!("状态 [{}]", 文本.id).as_str());

    let 记录: Vec<csv::StringRecord> = reader
        .records()
        .map(|记录| 记录.expect("CSV 行可解析"))
        .collect();
    assert_eq!(记录.len(), 2);
    assert_eq!(&记录[0][1], "complete");
    assert_eq!(&记录[1][1], "screen_out", "筛选结束同样导出并标注");
    assert_eq!(
        &记录[0][5],
        serde_json::to_string(&多选值)
            .expect("数组可序列化")
            .as_str(),
        "多选保持 JSON 数组，不用逗号拼接"
    );
    assert_eq!(&记录[0][6], "已答");
    assert_eq!(&记录[0][7], "'=cmd", "单元格公式前缀必须转义");
    assert_eq!(&记录[0][8], "已答");
    assert_eq!(&记录[1][5], "");
    assert_eq!(&记录[1][6], "适用未答");
    assert_eq!(&记录[1][7], "");
    assert_eq!(&记录[1][8], "规则跳过");
}

#[test]
fn 导出把表头与行批次分开且批次不带重复表头() {
    let (文档, 多选, ..) = 导出文档(None);
    let 表头 =
        String::from_utf8(encode_csv_header(&文档).expect("表头编码成功")).expect("表头是 UTF-8");
    assert_eq!(表头.lines().count(), 1, "表头只有一行");
    assert!(
        表头.starts_with("receipt_id,outcome,version_id,version_number,submitted_at"),
        "固定前缀列必须包含 outcome：{表头}"
    );

    let rows = vec![完成行(
        Uuid::new_v4(),
        时刻("2026-10-20T00:00:00Z"),
        答案(&[(多选.id, json!([]))]),
        状态(&[(多选.id, AnswerState::Unanswered)]),
    )];
    let 行 =
        String::from_utf8(encode_csv_rows(&文档, Uuid::new_v4(), 2, &rows).expect("行编码成功"))
            .expect("行是 UTF-8");
    assert_eq!(行.lines().count(), 1, "批次只包含数据行，不重复表头");
    assert!(
        行.starts_with(&format!("{},complete,", rows[0].receipt_id)),
        "{行}"
    );
}

#[test]
fn 均值更新在异号大数之间不溢出() {
    let mut 均值 = 0.0;
    update_mean(&mut 均值, 4.0, 1);
    assert_eq!(均值, 4.0);
    update_mean(&mut 均值, 6.0, 2);
    assert_eq!(均值, 5.0);
    update_mean(&mut 均值, 0.0, 3);
    assert!((均值 - 10.0 / 3.0).abs() < 1e-12, "{均值}");

    let mut 极值 = f64::MAX;
    update_mean(&mut 极值, -f64::MAX, 2);
    assert!(极值.is_finite(), "异号累加不能溢出为无穷：{极值}");
    assert_eq!(极值, 0.0);
}
