//! HTTP 输入只在边界解析一次。业务代码收到的是类型，不需要反复解析字符串。

use crate::{
    AppState,
    error::{AppError, AppResult},
};
use axum::{
    extract::{FromRequest, FromRequestParts, Json, Path, Query, Request},
    http::{StatusCode, request::Parts},
};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, DeserializeOwned},
};
use utoipa::{IntoParams, ToSchema};

pub(crate) struct ApiJson<T>(pub T);
pub(crate) struct ApiQuery<T>(pub T);
pub(crate) struct ApiPath<T>(pub T);

impl<T: DeserializeOwned + Send> FromRequest<AppState> for ApiJson<T> {
    type Rejection = AppError;
    async fn from_request(request: Request, state: &AppState) -> AppResult<Self> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|error| {
                if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    AppError::new(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "body_too_large",
                        "请求体超出允许的大小限制",
                    )
                } else {
                    // serde 错误可包含原始输入，不直接返回或记录。
                    AppError::new(
                        StatusCode::BAD_REQUEST,
                        "invalid_json",
                        "请求 JSON 格式或字段类型错误",
                    )
                }
            })
    }
}

impl<T: DeserializeOwned + Send> FromRequestParts<AppState> for ApiQuery<T> {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> AppResult<Self> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|_| {
                AppError::new(StatusCode::BAD_REQUEST, "invalid_query", "查询参数格式错误")
            })
    }
}

impl<T: DeserializeOwned + Send> FromRequestParts<AppState> for ApiPath<T> {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> AppResult<Self> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(|_| AppError::new(StatusCode::BAD_REQUEST, "invalid_path", "路径参数格式错误"))
    }
}

#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
pub(crate) struct Pagination {
    /// 页码，从 1 开始。
    #[serde(default, deserialize_with = "optional_u64")]
    pub(crate) page: Option<u64>,
    /// 每页 1..=100 条，默认 20。
    #[serde(default, deserialize_with = "optional_u64")]
    pub(crate) page_size: Option<u64>,
}

// 查询 DTO 用 flatten 嵌入 Pagination 时，Serde 先把值缓冲成字符串，
// 不再经过 serde_urlencoded 的数字解析；因此这里同时接受数字和十进制字符串。
fn optional_u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Decimal {
        Number(u64),
        Text(String),
    }

    match Option::<Decimal>::deserialize(deserializer)? {
        None => Ok(None),
        Some(Decimal::Number(value)) => Ok(Some(value)),
        Some(Decimal::Text(text)) => text
            .parse()
            .map(Some)
            .map_err(|_| de::Error::custom("分页参数必须是非负整数")),
    }
}

/// 一次解析完成的分页参数：业务层直接使用，不再手工重算偏移与页码。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PageBounds {
    pub page: u64,
    pub page_size: u64,
    pub offset: u64,
    pub limit: u64,
}

impl Pagination {
    pub fn bounds(&self) -> AppResult<PageBounds> {
        let page = self.page.unwrap_or(1);
        let page_size = self.page_size.unwrap_or(20);
        if page == 0 || !(1..=100).contains(&page_size) {
            return Err(AppError::validation(
                "page 至少为 1，page_size 必须在 1..=100",
            ));
        }
        let offset = (page - 1)
            .checked_mul(page_size)
            .filter(|offset| *offset <= i64::MAX as u64)
            .ok_or_else(|| AppError::validation("页码过大"))?;
        Ok(PageBounds {
            page,
            page_size,
            offset,
            limit: page_size,
        })
    }
}

/// 统一的分页响应结构；页码与页大小只在构造时从 `PageBounds` 复制。
#[derive(Serialize, ToSchema)]
pub(crate) struct Page<T: ToSchema> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

impl<T: ToSchema> Page<T> {
    pub(crate) fn new(items: Vec<T>, total: u64, bounds: PageBounds) -> Self {
        Self {
            items,
            total,
            page: bounds.page,
            page_size: bounds.page_size,
        }
    }
}
