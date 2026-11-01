use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct AppError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Value,
}

impl AppError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: json!({}),
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    pub fn body(&self) -> ErrorBody {
        ErrorBody {
            error: ErrorInner {
                code: self.code.to_string(),
                message: self.message.clone(),
                details: self.details.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub error: ErrorInner,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorInner {
    pub code: String,
    pub message: String,
    pub details: Value,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body())).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DB",
            format!("数据库错误：{e}"),
        )
    }
}
