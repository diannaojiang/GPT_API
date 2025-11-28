use crate::client::routing::select_clients_by_weight;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures::future;
use futures::stream::StreamExt;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use tracing::{error, info};

use crate::{
    client::proxy::{build_and_send_request, get_api_key},
    config::types::ClientConfig,
    db::check_and_rotate,
    db::records::log_non_streaming_request,
    handlers::utils::{
        apply_prefix_to_json, build_request_body_generic, filter_empty_messages, get_client_ip,
        process_messages, remove_think_tags,
    },
    models::requests::RequestPayload,
    state::app_state::AppState,
};

/// 统一处理所有请求的核心函式，包含模型查找、尝试和后备逻辑。
pub async fn handle_request_logic(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut payload: RequestPayload,
) -> Response {
    // 对 Chat 请求，预处理 messages
    if let RequestPayload::Chat(ref mut p) = payload {
        let processed_messages = process_messages(p.messages.clone());
        let filtered_messages = filter_empty_messages(processed_messages);
        p.messages = remove_think_tags(filtered_messages);
    }

    let mut current_model = payload.get_model().to_string();

    loop {
        payload.set_model(current_model.clone());
        let config_guard = app_state.config_manager.get_config_guard().await;
        let matching_clients = app_state
            .client_manager
            .find_matching_clients(&config_guard, &current_model)
            .await;
        let matching_clients = select_clients_by_weight(matching_clients);

        if matching_clients.is_empty() {
            let err_msg_str = format!("The model `{}` does not exist.", current_model);
            let err_msg = json!({ "error": err_msg_str });
            
            let mut response = (StatusCode::NOT_FOUND, Json(err_msg)).into_response();
            response.extensions_mut().insert(AccessLogMeta {
                model: current_model.clone(),
                error: Some("No matching clients found".to_string()),
            });
            return response;
        }

        let mut fallback_triggered = false;
        for client_config in matching_clients {
            // 所有请求都通过这个统一的派发函数
            let result = dispatch_request(&app_state, &headers, &payload, &client_config).await;

            match result {
                Ok(mut resp) => {
                    // 注入模型信息供日志中间件使用
                    resp.extensions_mut().insert(AccessLogMeta {
                        model: current_model.clone(),
                        error: None,
                    });
                    return resp;
                }
                Err(e) => {
                    debug!(
                        "Failed to process request with client {}: {:?}",
                        client_config.name, e
                    );
                    if let Some(fallback_model) = &client_config.fallback {
                        info!("Falling back to model: {}", fallback_model);
                        current_model = fallback_model.clone();
                        fallback_triggered = true;
                        break; // 跳出客户端循环，使用后备模型重新开始
                    }
                }
            }
        }

        if !fallback_triggered {
            // 如果尝试了所有客户端都失败了，并且没有触发后备，则返回错误
            let err_msg =
                json!({ "error": "All upstream providers failed for the requested model." });
            
            let mut response = (StatusCode::INTERNAL_SERVER_ERROR, Json(err_msg)).into_response();
            response.extensions_mut().insert(AccessLogMeta {
                model: current_model.clone(),
                error: Some("All upstream providers failed".to_string()),
            });
            return response;
        }
    }
}

/// 统一的请求派发函数，替代了原有的 `dispatch_*` 和 `try_*` 系列函数
async fn dispatch_request(
    app_state: &Arc<AppState>,
    headers: &HeaderMap,
    payload: &RequestPayload,
    client_config: &ClientConfig,
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    // 1. 根据请求类型确定 API 端点
    let (endpoint_path, is_chat) = match payload {
        RequestPayload::Chat(_) => ("chat/completions", true),
        RequestPayload::Completion(_) => ("completions", false),
        RequestPayload::Embedding(_) => ("embeddings", false),
        RequestPayload::Rerank(_) => ("rerank", false),
        RequestPayload::Score(_) => ("score", false),
        RequestPayload::Classify(_) => ("classify", false),
    };
    let url = format!(
        "{}/{}",
        client_config.base_url.trim_end_matches('/'),
        endpoint_path
    );
    let api_key = get_api_key(client_config, headers);

    // 2. 使用通用函数构建请求体
    let request_body = build_request_body_generic(payload, client_config, payload.is_streaming());

    // 3. 发送请求
    let response =
        build_and_send_request(app_state, client_config, &api_key, &url, &request_body).await?;

    // 4. 根据是否流式，调用不同的响应处理器
    if payload.is_streaming() {
        process_streaming_response(response, client_config, is_chat).await
    } else {
        process_non_streaming_response(
            app_state,
            headers,
            payload,
            client_config,
            &request_body,
            response,
        )
        .await
    }
}

/// 处理非流式响应
async fn process_non_streaming_response(
    app_state: &Arc<AppState>,
    headers: &HeaderMap,
    payload: &RequestPayload,
    client_config: &ClientConfig,
    request_body: &Value,
    response: reqwest::Response,
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    let mut response_body: Value = response.json().await?;

    if let Some(special_prefix) = &client_config.special_prefix {
        apply_prefix_to_json(
            &mut response_body,
            special_prefix,
            matches!(payload, RequestPayload::Chat(_)),
        );
    }

    // 在记录日志前检查并轮换数据库
    let app_state_clone = app_state.clone();
    let headers_clone = headers.clone();
    let payload_clone = payload.clone();
    let request_body_clone = request_body.clone();
    let response_body_clone = response_body.clone();
    tokio::spawn(async move {
        check_and_rotate(&app_state_clone).await;
        log_non_streaming_request(
            &app_state_clone,
            &headers_clone,
            &payload_clone,
            &request_body_clone,
            &response_body_clone,
        )
        .await;
    });

    Ok(Json(response_body).into_response())
}

/// 处理流式响应，并转换为 SSE Stream
async fn process_streaming_response(
    response: reqwest::Response,
    client_config: &ClientConfig,
    is_chat: bool,
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    let stream = response.bytes_stream();
    let special_prefix = client_config.special_prefix.clone().unwrap_or_default();
    let mut prefix_applied = false;

    // 根据是否为聊天请求，确定要修改的 JSON 字段路径
    let content_json_pointer = if is_chat {
        "/choices/0/delta/content"
    } else {
        "/choices/0/text"
    };

    let sse_stream = stream
        .map(|result| result.unwrap_or_default())
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
        .flat_map(|chunk| {
            let parts: Vec<String> = chunk.split("\n\n").map(String::from).collect();
            futures::stream::iter(parts)
        })
        .filter(|line| future::ready(!line.is_empty()))
        .map(move |line| {
            let mut event = Event::default();
            if line.starts_with("data:") {
                let data_str = line.strip_prefix("data:").unwrap().trim();
                if data_str == "[DONE]" {
                    event = event.data("[DONE]");
                } else if let Ok(mut value) = serde_json::from_str::<Value>(data_str) {
                    // 对第一个有效的消息块应用特殊前缀
                    if !prefix_applied && !special_prefix.is_empty() {
                        if let Some(delta_content) = value.pointer_mut(content_json_pointer) {
                            if let Some(s) = delta_content.as_str() {
                                if !s.is_empty() {
                                    *delta_content = json!(format!("{}{}", special_prefix, s));
                                    prefix_applied = true;
                                }
                            }
                        }
                    }
                    event = event.data(serde_json::to_string(&value).unwrap_or_default());
                } else {
                    event = event.data(data_str); // 保留原始数据以防 JSON 解析失败
                }
            } else {
                event = event.data(line);
            }
            Ok::<axum::response::sse::Event, Infallible>(event)
        });

    Ok(Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}
