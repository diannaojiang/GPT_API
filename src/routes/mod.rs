use crate::state::app_state::AppState;
use axum::{routing::get, Router};
use std::sync::Arc;

pub mod chat;
pub mod completions;
pub mod health;
pub mod models;

pub fn create_router(app_state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health::health_check))
        .route("/v1/models", get(models::get_models))
        .route(
            "/v1/chat/completions",
            get(chat::handle_chat_completion).post(chat::handle_chat_completion),
        )
        .route(
            "/v1/completions",
            get(completions::handle_completion).post(completions::handle_completion),
        )
        .with_state(app_state)
}
