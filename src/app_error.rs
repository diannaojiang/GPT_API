use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::fmt;

#[derive(Debug)]
pub enum AppError {
    ReqwestError(reqwest::Error),
    DatabaseError(sqlx::Error),
    ClientNotFound(String),
    UpstreamError(String),
    InvalidHeader(String),
    InternalServerError(String),
    ApiError(StatusCode, String, String), // status_code, message, error_type
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AppError::ReqwestError(err) => write!(f, "External request failed: {}", err),
            AppError::DatabaseError(err) => write!(f, "Database operation failed: {}", err),
            AppError::ClientNotFound(model) => write!(f, "The model `{}` does not exist.", model),
            AppError::UpstreamError(msg) => write!(f, "Upstream error: {}", msg),
            AppError::InvalidHeader(msg) => write!(f, "Invalid header: {}", msg),
            AppError::InternalServerError(msg) => write!(f, "Internal server error: {}", msg),
            AppError::ApiError(_, message, error_type) => write!(f, "{}: {}", error_type, message),
        }
    }
}

impl std::error::Error for AppError {}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message, error_type) = match self {
            AppError::ReqwestError(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("External request failed: {}", err),
                "ReqwestError".to_string(),
            ),
            AppError::DatabaseError(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database operation failed: {}", err),
                "DatabaseError".to_string(),
            ),
            AppError::ClientNotFound(model) => (
                StatusCode::NOT_FOUND,
                format!("The model `{}` does not exist.", model),
                "NotFoundError".to_string(),
            ),
            AppError::UpstreamError(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                msg,
                "UpstreamError".to_string(),
            ),
            AppError::InvalidHeader(msg) => {
                (StatusCode::BAD_REQUEST, msg, "InvalidHeader".to_string())
            }
            AppError::InternalServerError(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                msg,
                "InternalServerError".to_string(),
            ),
            AppError::ApiError(status, message, error_type) => (status, message, error_type),
        };

        let body = Json(json!({ "error": error_message, "error_type": error_type }));
        (status, body).into_response()
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::ReqwestError(err)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::DatabaseError(err)
    }
}
