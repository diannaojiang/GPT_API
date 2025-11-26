use axum::{extract::State, http::HeaderMap, response::Response, Json};
use std::sync::Arc;
use tracing::info;

use crate::{
    handlers::chat_handler::handle_request_logic,
    models::requests::{ChatCompletionRequest, RequestPayload},
    state::app_state::AppState,
};

/// 处理聊天完成请求
pub async fn handle_chat_completion(
    state: State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Json<ChatCompletionRequest>,
) -> Response {
    info!(
        "Handling chat completion request for model: {}",
        payload.model
    );
    handle_request_logic(state, headers, RequestPayload::Chat(payload.0)).await
}
