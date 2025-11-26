use axum::{response::IntoResponse, Json};
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    status: String,
}

/// Health check endpoint
pub async fn health_check() -> impl IntoResponse {
    let response = HealthResponse {
        status: "ok".to_string(),
    };
    Json(response)
}
