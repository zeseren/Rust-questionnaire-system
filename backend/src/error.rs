//! 可预期的业务失败返回稳定错误码；数据库细节不出现在响应或诊断日志中。

use axum::{
    Json,
    http::StatusCode,
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
}

impl AppError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            question_id: None,
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

    pub fn internal(_error: impl std::fmt::Display) -> Self {
        // ORM 错误可能包含数据库中的原始值，不能直接记录 error.to_string()。
        tracing::error!("internal operation failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "服务暂时无法完成请求",
        )
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: ErrorDetail {
                    code: self.code.to_owned(),
                    message: self.message,
                    question_id: self.question_id,
                },
            }),
        )
            .into_response()
    }
}

impl From<sea_orm::DbErr> for AppError {
    fn from(error: sea_orm::DbErr) -> Self {
        Self::internal(error)
    }
}
impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        Self::internal(error)
    }
}
impl From<serde_json::Error> for AppError {
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
