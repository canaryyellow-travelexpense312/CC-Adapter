use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::types::anthropic::{ErrorDetail, ErrorResponse};

/// 統一的應用程式錯誤型別，回應格式符合 Anthropic API 的錯誤結構
/// Unified application error type; response format matches Anthropic API error structure
pub struct AppError {
    pub status: StatusCode,
    pub error_type: String,
    pub message: String,
}

impl AppError {
    /// 400 錯誤請求
    /// 400 Bad Request
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error_type: "invalid_request_error".to_string(),
            message: msg.into(),
        }
    }

    /// 500 內部伺服器錯誤
    /// 500 Internal Server Error
    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error_type: "api_error".to_string(),
            message: msg.into(),
        }
    }

    /// 功能尚未實作（以 400 回應）
    /// Feature not yet implemented (responds with 400)
    #[allow(dead_code)]
    pub fn not_implemented(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error_type: "invalid_request_error".to_string(),
            message: msg.into(),
        }
    }
}

/// 轉換為 Anthropic 格式的 JSON 錯誤回應
/// Convert into an Anthropic-formatted JSON error response
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = ErrorResponse {
            error_type: "error".to_string(),
            error: ErrorDetail {
                error_type: self.error_type,
                message: self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::internal(format!("{:#}", err))
    }
}
