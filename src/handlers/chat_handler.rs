use crate::app_error::AppError;
use crate::client::routing::select_clients_by_weight;
use crate::models::AccessLogMeta;
use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, info};

use crate::{
    client::proxy::{build_and_send_request, get_api_key},
    config::types::ClientConfig,
    db::records::log_non_streaming_request,
    handlers::stream_handler::{extract_error_msg, process_streaming_response},
    handlers::utils::{
        apply_prefix_to_json, build_request_body_generic, filter_empty_messages, get_client_ip,
        process_messages, remove_think_tags, truncate_json,
    },
    models::requests::RequestPayload,
    state::app_state::AppState,
};

fn prepare_chat_request(payload: &mut RequestPayload) {
    if let RequestPayload::Chat(ref mut p) = payload {
        let processed_messages = process_messages(p.messages.clone());
        let filtered_messages = filter_empty_messages(processed_messages);
        p.messages = remove_think_tags(filtered_messages);
    }
}

async fn resolve_client_chain(
    app_state: &Arc<AppState>,
    model_name: &str,
) -> Result<Vec<ClientConfig>, AppError> {
    let config_guard = app_state.config_manager.get_config_guard().await;
    let matching_clients = app_state
        .client_manager
        .find_matching_clients(&config_guard, model_name)
        .await;
    let matching_clients = select_clients_by_weight(matching_clients);

    if matching_clients.is_empty() {
        Err(AppError::ClientNotFound(model_name.to_string()))
    } else {
        Ok(matching_clients)
    }
}

async fn execute_client_chain(
    app_state: &Arc<AppState>,
    headers: &HeaderMap,
    addr: Option<SocketAddr>,
    payload: &RequestPayload,
    clients: Vec<ClientConfig>,
    current_model_name: &str,
) -> Result<Response, Option<String>> {
    let mut last_response: Option<Response> = None;

    for client_config in clients {
        let result = dispatch_request(app_state, headers, addr, payload, &client_config).await;

        match result {
            Ok(mut resp) => {
                let status = resp.status();

                // 1. 成功 (2xx) -> 直接返回
                if status.is_success() {
                    resp.extensions_mut().insert(AccessLogMeta {
                        model: current_model_name.to_string(),
                        error: None,
                        request_body: None,
                    });
                    return Ok(resp);
                }

                // 2. 客户端错误 (4xx) -> 认为是业务错误，不重试，直接透传
                if status.is_client_error() {
                    if let Some(meta) = resp.extensions_mut().get_mut::<AccessLogMeta>() {
                        meta.model = current_model_name.to_string();
                    } else {
                        resp.extensions_mut().insert(AccessLogMeta {
                            model: current_model_name.to_string(),
                            error: Some(format!("Upstream client error: {}", status)),
                            request_body: None,
                        });
                    }
                    return Ok(resp);
                }

                // 3. 服务端错误 (5xx) -> 检查是否有 Fallback
                if status.is_server_error() {
                    debug!(
                        "Client {} failed with status {}. Checking fallback...",
                        client_config.name, status
                    );

                    if let Some(fallback_model) = &client_config.fallback {
                        info!("Falling back to model: {}", fallback_model);
                        return Err(Some(fallback_model.clone()));
                    }

                    last_response = Some(resp);
                }
            }
            Err(e) => {
                debug!(
                    "Failed to process request with client {}: {}",
                    client_config.name, e
                );
                if let Some(fallback_model) = &client_config.fallback {
                    info!("Falling back to model: {}", fallback_model);
                    return Err(Some(fallback_model.clone()));
                }
                last_response = Some(e.into_response());
            }
        }
    }

    if let Some(mut resp) = last_response {
        if let Some(meta) = resp.extensions_mut().get_mut::<AccessLogMeta>() {
            meta.model = current_model_name.to_string();
        } else {
            resp.extensions_mut().insert(AccessLogMeta {
                model: current_model_name.to_string(),
                error: Some("All upstream providers failed (forwarding last error)".to_string()),
                request_body: None,
            });
        }
        return Ok(resp);
    }

    Err(None)
}

/// 统一处理所有请求的核心逻辑
///
/// 该函数实现了请求的完整生命周期管理：
/// 1. **预处理**: 对 Chat 请求的消息进行清洗（去除空白、Think标签等）。
/// 2. **模型匹配**: 查找配置中对应的后端客户端。
/// 3. **负载均衡**: 根据权重选择具体的客户端。
/// 4. **故障转移 (Failover)**: 当首选客户端失败（5xx错误）时，自动尝试 fallback 模型。
/// 5. **日志记录**: 无论成功失败，都通过 AccessLogMeta 注入详细信息供中间件记录。
pub async fn handle_request_logic(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    addr: Option<SocketAddr>,
    mut payload: RequestPayload,
) -> Response {
    // 对 Chat 请求，预处理 messages
    prepare_chat_request(&mut payload);

    let mut current_model = payload.get_model().to_string();

    loop {
        payload.set_model(current_model.clone());
        let matching_clients = match resolve_client_chain(&app_state, &current_model).await {
            Ok(clients) => clients,
            Err(e) => return e.into_response(),
        };

        let matching_client_names: Vec<String> =
            matching_clients.iter().map(|c| c.name.clone()).collect();

        match execute_client_chain(
            &app_state,
            &headers,
            addr,
            &payload,
            matching_clients,
            &current_model,
        )
        .await
        {
            Ok(response) => return response,
            Err(Some(fallback_model)) => {
                current_model = fallback_model;
                continue;
            }
            Err(None) => {
                // All failed
                let error_message = format!(
                    "All upstream providers failed for the requested model. Tried clients: {:?}",
                    matching_client_names
                );

                let mut response =
                    AppError::InternalServerError(error_message.clone()).into_response();

                let payload_value = serde_json::to_value(&payload)
                    .unwrap_or(json!({"error": "failed to serialize payload"}));
                let log_body =
                    serde_json::to_string(&truncate_json(&payload_value)).unwrap_or_default();

                response.extensions_mut().insert(AccessLogMeta {
                    model: current_model.clone(),
                    error: Some(error_message),
                    request_body: Some(log_body),
                });
                return response;
            }
        }
    }
}

/// 统一的请求派发函数
async fn dispatch_request(
    app_state: &Arc<AppState>,
    headers: &HeaderMap,
    addr: Option<SocketAddr>,
    payload: &RequestPayload,
    client_config: &ClientConfig,
) -> Result<Response, AppError> {
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
    let response = build_and_send_request(app_state, client_config, &api_key, &url, &request_body)
        .await
        .map_err(|e| match e.downcast::<reqwest::Error>() {
            Ok(req_err) => AppError::from(*req_err),
            Err(original_err) => AppError::InternalServerError(original_err.to_string()),
        })?;

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
) -> Result<Response, AppError> {
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
