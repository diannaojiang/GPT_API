use crate::client::routing::select_clients_by_weight;
use crate::middleware::access_log::AccessLogMeta;
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
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, info};

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
    addr: Option<SocketAddr>,
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
            let err_msg = json!({
                "error": format!("The model `{}` does not exist.", current_model),
                "error_type": "Input Validation Error"
            });

            let mut response = (StatusCode::UNPROCESSABLE_ENTITY, Json(err_msg)).into_response();
            response.extensions_mut().insert(AccessLogMeta {
                model: current_model.clone(),
                error: Some("No matching clients found".to_string()),
            });
            return response;
        }

        let mut last_response: Option<Response> = None;
        let mut fallback_triggered = false;

        for client_config in matching_clients {
            // 所有请求都通过这个统一的派发函数
            // 注意：dispatch_request 现在即使上游返回 4xx/5xx 也会返回 Ok(Response)
            let result =
                dispatch_request(&app_state, &headers, addr, &payload, &client_config).await;

            match result {
                Ok(mut resp) => {
                    let status = resp.status();

                    // 1. 成功 (2xx) -> 直接返回
                    if status.is_success() {
                        resp.extensions_mut().insert(AccessLogMeta {
                            model: current_model.clone(),
                            error: None,
                        });
                        return resp;
                    }

                    // 2. 客户端错误 (4xx) -> 认为是业务错误，不重试，直接透传
                    if status.is_client_error() {
                        // 注入 Log Meta (尝试从 Header 或状态推断，不读取 Body 以免消耗流)
                        resp.extensions_mut().insert(AccessLogMeta {
                            model: current_model.clone(),
                            error: Some(format!("Upstream client error: {}", status)),
                        });
                        return resp;
                    }

                    // 3. 服务端错误 (5xx) -> 检查是否有 Fallback
                    if status.is_server_error() {
                        debug!(
                            "Client {} failed with status {}. Checking fallback...",
                            client_config.name, status
                        );

                        // 如果有后备模型，则触发后备逻辑
                        if let Some(fallback_model) = &client_config.fallback {
                            info!("Falling back to model: {}", fallback_model);
                            current_model = fallback_model.clone();
                            fallback_triggered = true;
                            // 保存这次的错误响应，万一 fallback 也失败，至少能返回这个（或者返回 fallback 的失败）
                            // 但由于我们要 break loop 重来，last_response 在这里赋值其实会被外层的 loop 覆盖。
                            // 真正的逻辑是：break inner loop, continue outer loop.
                            break;
                        }

                        // 如果没有后备，或者这就是最后一个尝试的 client，
                        // 我们将这个响应保存为“最后一次失败”，继续尝试下一个 client (负载均衡/重试同模型)
                        // 但目前逻辑是 `select_clients_by_weight` 返回的是列表，我们遍历列表。
                        // 如果这个 client 失败且没 fallback，我们应该尝试列表中的下一个吗？
                        // 通常负载均衡是选一个。这里 matching_clients 可能是多个（如果配置了多个同名模型）。
                        // 假设我们尝试下一个同名模型的 client。
                        last_response = Some(resp);
                    }
                }
                Err(e) => {
                    // 网络层错误（连接失败、超时等），由 reqwest 抛出
                    debug!(
                        "Failed to process request with client {}: {:?}",
                        client_config.name, e
                    );
                    // 如果有 fallback，优先 fallback
                    if let Some(fallback_model) = &client_config.fallback {
                        info!("Falling back to model: {}", fallback_model);
                        current_model = fallback_model.clone();
                        fallback_triggered = true;
                        break;
                    }
                    // 否则继续尝试下一个 client
                }
            }
        }

        // 如果触发了 fallback，跳出当前 client 循环，使用新模型重新开始
        if fallback_triggered {
            continue;
        }

        // 如果所有 client 都尝试过且失败了，返回最后一次的错误响应
        if let Some(mut resp) = last_response {
            resp.extensions_mut().insert(AccessLogMeta {
                model: current_model.clone(),
                error: Some("All upstream providers failed (forwarding last error)".to_string()),
            });
            return resp;
        }

        // 如果连 last_response 都没有（例如全是网络错误），返回通用错误
        let err_msg = json!({
            "error": "All upstream providers failed for the requested model.",
            "error_type": "upstream_error"
        });

        let mut response = (StatusCode::INTERNAL_SERVER_ERROR, Json(err_msg)).into_response();
        response.extensions_mut().insert(AccessLogMeta {
            model: current_model.clone(),
            error: Some("All upstream providers failed".to_string()),
        });
        return response;
    }
}

/// 统一的请求派发函数
async fn dispatch_request(
    app_state: &Arc<AppState>,
    headers: &HeaderMap,
    addr: Option<SocketAddr>,
    payload: &RequestPayload,
    client_config: &ClientConfig,
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
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

    let request_body = build_request_body_generic(payload, client_config, payload.is_streaming());

    // 这里 build_and_send_request 现在返回 Ok(response) 即使状态码是 4xx/5xx
    let response =
        build_and_send_request(app_state, client_config, &api_key, &url, &request_body).await?;

    // 核心修改：检查是否应该进入流式处理
    // 只有当用户请求流式 且 响应状态码为成功时，才进入流式处理
    if payload.is_streaming() {
        process_streaming_response(response, client_config, is_chat).await
    } else {
        process_non_streaming_response(
            app_state,
            headers,
            addr,
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
    addr: Option<SocketAddr>,
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
    let client_ip = get_client_ip(headers, addr);

    tokio::spawn(async move {
        check_and_rotate(&app_state_clone).await;
        log_non_streaming_request(
            &app_state_clone,
            &headers_clone,
            &payload_clone,
            &request_body_clone,
            &response_body_clone,
            client_ip,
        )
        .await;
    });

    Ok(Json(response_body).into_response())
}

// process_streaming_response 也需要修改以处理错误状态码
async fn process_streaming_response(
    response: reqwest::Response,
    client_config: &ClientConfig,
    is_chat: bool,
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    // 如果状态码不成功，直接作为普通 JSON 返回，不要包装成 SSE
    if !response.status().is_success() {
        let status = response.status();
        // 尝试解析为 JSON，如果不是 JSON 则作为文本返回
        // 这里我们尽力而为，目的是透传上游的 body
        let body_bytes = response.bytes().await?;
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or_else(|_| {
            json!({
                "error": String::from_utf8_lossy(&body_bytes).to_string(),
                "error_type": "upstream_error"
            })
        });
        return Ok((status, Json(body_json)).into_response());
    }

    let stream = response.bytes_stream();
    let special_prefix = client_config.special_prefix.clone().unwrap_or_default();
    let mut prefix_applied = false;

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
                    event = event.data(data_str);
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
