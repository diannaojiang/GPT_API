use axum::{extract::State, http::HeaderMap, response::Response, Json};
use std::sync::Arc;
use tracing::info;

use crate::{
    handlers::chat_handler::handle_request_logic,
    models::requests::{EmbeddingRequest, RequestPayload},
    state::app_state::AppState,
};

pub async fn handle_embeddings(
    state: State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Json<EmbeddingRequest>,
) -> Response {
    info!("Handling embeddings request for model: {}", payload.model);
    handle_request_logic(state, headers, RequestPayload::Embedding(payload.0)).await
}
