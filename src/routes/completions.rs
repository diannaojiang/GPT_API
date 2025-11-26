use axum::{extract::State, http::HeaderMap, response::Response, Json};
use std::sync::Arc;
use tracing::info;

use crate::{
    handlers::chat_handler::handle_request_logic,
    models::requests::{CompletionRequest, RequestPayload},
    state::app_state::AppState,
};

/// 处理完成请求
pub async fn handle_completion(
    state: State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Json<CompletionRequest>,
) -> Response {
    info!("Handling completion request for model: {}", payload.model);
    handle_request_logic(state, headers, RequestPayload::Completion(payload.0)).await
}
