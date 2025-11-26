use crate::{handlers::audio_handler, state::app_state::AppState};
use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

pub mod chat;
pub mod completions;
pub mod general;
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
        .route(
            "/v1/embeddings",
            get(general::handle_embeddings).post(general::handle_embeddings),
        )
        .route(
            "/v1/rerank",
            get(general::handle_rerank).post(general::handle_rerank),
        )
        .route(
            "/rerank",
            get(general::handle_rerank).post(general::handle_rerank),
        )
        .route(
            "/score",
            get(general::handle_score).post(general::handle_score),
        )
        .route(
            "/classify",
            get(general::handle_classify).post(general::handle_classify),
        )
        .route(
            "/v1/audio/transcriptions",
            post(audio_handler::handle_audio_transcription),
        )
        .route(
            "/v1/audio/translations",
            post(audio_handler::handle_audio_translation),
        )
        .with_state(app_state)
}
