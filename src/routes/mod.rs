use crate::state::app_state::AppState;
use axum::{
    http::Method,
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub mod audio;
pub mod chat;
pub mod completions;
pub mod embeddings;
pub mod extras;
pub mod health;
pub mod models;

pub fn create_router(app_state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_origin(Any)
        .allow_headers(Any);

    Router::new()
        .route("/health", get(health::health_check))
        .route("/v1/models", get(models::get_models))
        .route("/v1/chat/completions", post(chat::handle_chat_completion))
        .route("/v1/completions", post(completions::handle_completion))
        .route("/v1/embeddings", post(embeddings::handle_embeddings))
        .route("/v1/rerank", post(extras::handle_rerank))
        .route("/rerank", post(extras::handle_rerank))
        .route("/score", post(extras::handle_score))
        .route("/classify", post(extras::handle_classify))
        .route(
            "/v1/audio/transcriptions",
            post(audio::handle_audio_transcription),
        )
        .route(
            "/v1/audio/translations",
            post(audio::handle_audio_translation),
        )
        .layer(cors)
        .with_state(app_state)
}
