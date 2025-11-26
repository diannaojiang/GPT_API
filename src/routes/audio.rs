use axum::{
    extract::{Multipart, State},
    http::HeaderMap,
    response::Response,
};
use std::sync::Arc;
use tracing::info;

use crate::{handlers::audio_handler, state::app_state::AppState};

pub async fn handle_audio_transcription(
    state: State<Arc<AppState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    info!("Handling audio transcription request");
    audio_handler::handle_audio_transcription(state, headers, multipart).await
}

pub async fn handle_audio_translation(
    state: State<Arc<AppState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    info!("Handling audio translation request");
    audio_handler::handle_audio_translation(state, headers, multipart).await
}
