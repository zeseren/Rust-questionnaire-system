//! 跨模块单元测试：HTTP 边界解析、CSV 公式防护与问卷状态换算。

use crate::entities::SurveyStatus;
use crate::http::{PageBounds, Pagination};
use crate::reports::formula_safe;
use chrono::{Duration, Utc};
use std::borrow::Cow;

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
