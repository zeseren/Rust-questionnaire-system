//! HTTP 输入只在边界解析一次。业务代码收到的是类型，不需要反复解析字符串。

use crate::{
    AppState,
    error::{AppError, AppResult},
};
use axum::{
    extract::{FromRequest, FromRequestParts, Json, Path, Query, Request},
    http::{StatusCode, request::Parts},
};
use serde::{Deserialize, de::DeserializeOwned};
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
    pub page: Option<u64>,
    /// 每页 1..=100 条，默认 20。
    pub page_size: Option<u64>,
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
