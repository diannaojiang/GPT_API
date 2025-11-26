use axum::{extract::State, http::HeaderMap, response::Response, Json};
use std::sync::Arc;
use tracing::info;

use crate::{
    handlers::chat_handler::handle_request_logic,
    models::requests::{
        ClassifyRequest, RequestPayload, RerankRequest, ScoreRequest,
    },
    state::app_state::AppState,
};

pub async fn handle_rerank(
    state: State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Json<RerankRequest>,
) -> Response {
    info!("Handling rerank request for model: {}", payload.model);
    handle_request_logic(state, headers, RequestPayload::Rerank(payload.0)).await
}

pub async fn handle_score(
    state: State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Json<ScoreRequest>,
) -> Response {
    info!("Handling score request for model: {}", payload.model);
    handle_request_logic(state, headers, RequestPayload::Score(payload.0)).await
}

pub async fn handle_classify(
    state: State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Json<ClassifyRequest>,
) -> Response {
    info!("Handling classify request for model: {}", payload.model);
    handle_request_logic(state, headers, RequestPayload::Classify(payload.0)).await
}
