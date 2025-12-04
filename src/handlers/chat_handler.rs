use crate::client::routing::select_clients_by_weight;
use crate::models::AccessLogMeta;
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
        process_messages, remove_think_tags, truncate_json,
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
                request_body: None,
            });
            return response;
        }

        let matching_client_names: Vec<String> =
            matching_clients.iter().map(|c| c.name.clone()).collect();
        let mut last_response: Option<Response> = None;
        let mut fallback_triggered = false;

        for client_config in matching_clients {
            // 所有请求都通过这个统一的派发函数
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
                            request_body: None,
                        });
                        return resp;
                    }

                    // 2. 客户端错误 (4xx) -> 认为是业务错误，不重试，直接透传
                    if status.is_client_error() {
                        // 不要在这里注入 AccessLogMeta，因为下层函数已经注入了更详细的信息
                        // 如果这里再次注入，会覆盖掉下层的详细信息
                        // 但我们需要注入 model 信息，因为它在下层可能只是 "-"
                        // 我们可以尝试获取已有的 meta，更新 model，然后再放回去
                        if let Some(meta) = resp.extensions_mut().get_mut::<AccessLogMeta>() {
                            meta.model = current_model.clone();
                        } else {
                            // 如果下层没注入（理论上不应该发生），补一个
                            resp.extensions_mut().insert(AccessLogMeta {
                                model: current_model.clone(),
                                error: Some(format!("Upstream client error: {}", status)),
                                request_body: None,
                            });
                        }
                        return resp;
                    }

                    // 3. 服务端错误 (5xx) -> 检查是否有 Fallback
                    if status.is_server_error() {
                        debug!(
                            "Client {} failed with status {}. Checking fallback...",
                            client_config.name, status
                        );

                        if let Some(fallback_model) = &client_config.fallback {
                            info!("Falling back to model: {}", fallback_model);
                            current_model = fallback_model.clone();
                            fallback_triggered = true;
                            break;
                        }

                        last_response = Some(resp);
                    }
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
                        break;
                    }
                }
            }
        }

        if fallback_triggered {
            continue;
        }

        if let Some(mut resp) = last_response {
            // 同样，尝试更新 model 信息
            if let Some(meta) = resp.extensions_mut().get_mut::<AccessLogMeta>() {
                meta.model = current_model.clone();
            } else {
                resp.extensions_mut().insert(AccessLogMeta {
                    model: current_model.clone(),
                    error: Some(
                        "All upstream providers failed (forwarding last error)".to_string(),
                    ),
                    request_body: None,
                });
            }
            return resp;
        }

        let error_message = format!(
            "All upstream providers failed for the requested model. Tried clients: {:?}",
            matching_client_names
        );

        let err_msg = json!({
            "error": error_message,
            "error_type": "upstream_error"
        });

        let mut response = (StatusCode::INTERNAL_SERVER_ERROR, Json(err_msg)).into_response();
        
        // Serialize payload for logging
        let payload_value = serde_json::to_value(&payload).unwrap_or(json!({"error": "failed to serialize payload"}));
        let log_body = serde_json::to_string(&truncate_json(&payload_value)).unwrap_or_default();

        response.extensions_mut().insert(AccessLogMeta {
            model: current_model.clone(),
            error: Some(error_message),
            request_body: Some(log_body),
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
        process_streaming_response(response, client_config, is_chat, &request_body).await
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
    let status = response.status();
    let mut response_body: Value = response.json().await?;

    // 如果是错误响应，尝试提取错误信息并注入日志元数据
    let error_msg = if !status.is_success() {
        extract_error_msg(&response_body)
    } else {
        None
    };

    if status.is_success() {
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
    }

    let mut resp = Json(response_body).into_response();
    // 如果有状态码不一致（例如 Json 可能会默认 200，或者我们需要显式设置 status）， Axum 的 Json extractor 通常会设置 200。
    // 我们需要手动把 reqwest 的 status 设置回去。
    *resp.status_mut() = status;

    if let Some(msg) = error_msg {
        let log_body = serde_json::to_string(&truncate_json(request_body)).unwrap_or_default();
        resp.extensions_mut().insert(AccessLogMeta {
            model: "-".to_string(), // Placeholder, will be updated by handle_request_logic
            error: Some(msg),
            request_body: Some(log_body),
        });
    }

    Ok(resp)
}

async fn process_streaming_response(
    response: reqwest::Response,
    client_config: &ClientConfig,
    is_chat: bool,
    request_body: &Value,
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    // 如果状态码不成功，直接作为普通 JSON 返回，不要包装成 SSE
    if !response.status().is_success() {
        let status = response.status();
        let body_bytes = response.bytes().await?;
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or_else(|_| {
            json!({
                "error": String::from_utf8_lossy(&body_bytes).to_string(),
                "error_type": "upstream_error"
            })
        });

        let error_msg = extract_error_msg(&body_json);

        let mut resp = (status, Json(body_json)).into_response();

        if let Some(msg) = error_msg {
            let log_body = serde_json::to_string(&truncate_json(request_body)).unwrap_or_default();
            resp.extensions_mut().insert(AccessLogMeta {
                model: "-".to_string(),
                error: Some(msg),
                request_body: Some(log_body),
            });
        }

        return Ok(resp);
    }
    // ... (rest of streaming logic) ...
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

fn extract_error_msg(body: &Value) -> Option<String> {
    if let Some(error) = body.get("error") {
        if let Some(msg) = error.get("message").and_then(|v| v.as_str()) {
            return Some(msg.to_string());
        }
        if let Some(msg) = error.as_str() {
            return Some(msg.to_string());
        }
    }
    // 如果是扁平结构 {"error": "msg", ...}
    if let Some(msg) = body.get("error").and_then(|v| v.as_str()) {
        return Some(msg.to_string());
    }
    // 尝试直接把 body 转字符串（如果是简单的 {"error": ...}）
    // 或者如果不包含 error 字段，但 status 错了，就返回 body 的紧凑字符串
    Some(body.to_string())
}
