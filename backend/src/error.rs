//! 可预期的业务失败返回稳定错误码；数据库细节不出现在响应或诊断日志中。

use axum::{
    Json,
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

pub(crate) type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ErrorDetail {
    pub code: String,
    pub message: String,
    pub question_id: Option<Uuid>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct AppError {
    status: StatusCode,
    code: &'static str,
    message: String,
    question_id: Option<Uuid>,
    /// 只有内部错误生成；日志与响应头用同一标识定位一次故障。
    error_id: Option<String>,
}

impl AppError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            question_id: None,
            error_id: None,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            message,
        )
    }

    pub fn field(question_id: Uuid, message: impl Into<String>) -> Self {
        Self {
            question_id: Some(question_id),
            ..Self::validation(message)
        }
    }

    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "资源不存在或不可访问")
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "请重新登录")
    }

    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", "没有执行此操作的权限")
    }

    /// 内部错误的统一出口：记录错误类别与发起位置，并生成可回传检索的标识。
    ///
    /// 原始错误可能包含数据库中的原始值或 SQL 文本，绝不记录 `error.to_string()`；
    /// `#[track_caller]` 让位置指向真正发起操作的位置，而不是本函数。
    #[track_caller]
    pub fn internal(error: impl std::fmt::Display) -> Self {
        let error_id = Uuid::new_v4().simple().to_string();
        let error_id = error_id[..16].to_owned();
        tracing::error!(
            error_id = %error_id,
            error_kind = std::any::type_name_of_val(&error),
            location = %std::panic::Location::caller(),
            "internal operation failed"
        );
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: "服务暂时无法完成请求".to_owned(),
            question_id: None,
            error_id: Some(error_id),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let Self {
            status,
            code,
            message,
            question_id,
            error_id,
        } = self;
        let body = Json(ErrorResponse {
            error: ErrorDetail {
                code: code.to_owned(),
                message,
                question_id,
            },
        });
        let mut response = (status, body).into_response();
        // 响应头让客户端可以回传标识，日志侧凭同一标识定位故障；响应体契约不变。
        if let Some(error_id) = error_id
            && let Ok(value) = HeaderValue::from_str(&error_id)
        {
            response.headers_mut().insert("x-error-id", value);
        }
        response
    }
}

impl From<sea_orm::DbErr> for AppError {
    #[track_caller]
    fn from(error: sea_orm::DbErr) -> Self {
        Self::internal(error)
    }
}
impl From<sqlx::Error> for AppError {
    #[track_caller]
    fn from(error: sqlx::Error) -> Self {
        Self::internal(error)
    }
}
impl From<serde_json::Error> for AppError {
    #[track_caller]
    fn from(error: serde_json::Error) -> Self {
        Self::internal(error)
    }
}
impl From<crate::domain::ValidationError> for AppError {
    fn from(error: crate::domain::ValidationError) -> Self {
        match error.question_id {
            Some(id) => Self::field(id, error.message),
            None => Self::validation(error.message),
        }
    }
}
