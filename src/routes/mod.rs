use crate::state::app_state::AppState;
use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

pub mod audio;
pub mod chat;
pub mod completions;
pub mod embeddings;
pub mod extras;
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
            get(embeddings::handle_embeddings).post(embeddings::handle_embeddings),
        )
        .route(
            "/v1/rerank",
            get(extras::handle_rerank).post(extras::handle_rerank),
        )
        .route(
            "/rerank",
            get(extras::handle_rerank).post(extras::handle_rerank),
        )
        .route(
            "/score",
            get(extras::handle_score).post(extras::handle_score),
        )
        .route(
            "/classify",
            get(extras::handle_classify).post(extras::handle_classify),
        )
        .route(
            "/v1/audio/transcriptions",
            post(audio::handle_audio_transcription),
        )
        .route(
            "/v1/audio/translations",
            post(audio::handle_audio_translation),
        )
        .with_state(app_state)
}
