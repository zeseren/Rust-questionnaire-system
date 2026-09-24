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
    Deserialize, Deserializer,
    de::{self, DeserializeOwned},
};
use std::fmt;
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
                        "请求体不能超过 1 MiB",
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
    pub page: Option<u64>,
    /// 每页 1..=100 条，默认 20。
    #[serde(default, deserialize_with = "optional_u64")]
    pub page_size: Option<u64>,
}

// 查询 DTO 用 flatten 嵌入 Pagination 时，Serde 先把值缓冲成字符串，
// 不再经过 serde_urlencoded 的数字解析；因此这里同时接受数字和十进制字符串。
fn optional_u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    struct Visitor;
    impl de::Visitor<'_> for Visitor {
        type Value = u64;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("非负整数")
        }
        fn visit_u64<E: de::Error>(self, value: u64) -> Result<u64, E> {
            Ok(value)
        }
        fn visit_str<E: de::Error>(self, value: &str) -> Result<u64, E> {
            // 只接受 ASCII 十进制数字，拒绝 `+1` 这类 str::parse 允许的写法。
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(E::invalid_value(de::Unexpected::Str(value), &self));
            }
            value
                .parse()
                .map_err(|_| E::invalid_value(de::Unexpected::Str(value), &self))
        }
    }
    deserializer.deserialize_any(Visitor).map(Some)
}

impl Pagination {
    pub fn bounds(&self) -> AppResult<(u64, u64)> {
        let page = self.page.unwrap_or(1);
        let size = self.page_size.unwrap_or(20);
        if page == 0 || !(1..=100).contains(&size) {
            return Err(AppError::validation(
                "page 至少为 1，page_size 必须在 1..=100",
            ));
        }
        let offset = (page - 1)
            .checked_mul(size)
            .filter(|offset| *offset <= i64::MAX as u64)
            .ok_or_else(|| AppError::validation("页码过大"))?;
        Ok((offset, size))
    }
}
