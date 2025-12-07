use crate::app_error::AppError;
use crate::models::AccessLogMeta;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;

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

fn is_empty_value(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(arr) => arr.is_empty(),
        Value::Object(obj) => obj.is_empty(),
        _ => false,
    }
}

/// 统一处理所有请求的核心逻辑
///
/// 该函数实现了请求的完整生命周期管理，现已委托给 `DispatcherService` 处理路由和故障转移。
pub async fn handle_request_logic(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    addr: Option<SocketAddr>,
    mut payload: RequestPayload,
) -> Response {
    // 1. 输入验证
    match &payload {
        RequestPayload::Chat(p) => {
            if p.messages.is_empty() {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": "Request param messages not arr or arr is empty",
                        "error_type": "Input Validation Error"
                    })),
                )
                    .into_response();
            }
        }
        RequestPayload::Completion(p) => {
            if p.prompt.is_empty() {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": "Request param prompt is empty",
                        "error_type": "Input Validation Error"
                    })),
                )
                    .into_response();
            }
        }
        RequestPayload::Embedding(p) => {
            if is_empty_value(&p.input) {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": "Request param input is empty",
                        "error_type": "Input Validation Error"
                    })),
                )
                    .into_response();
            }
        }
        RequestPayload::Rerank(p) => {
            if p.query.is_empty() || p.documents.is_empty() {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": "Request param query or documents is empty",
                        "error_type": "Input Validation Error"
                    })),
                )
                    .into_response();
            }
        }
        RequestPayload::Score(p) => {
            if is_empty_value(&p.text_1) || is_empty_value(&p.text_2) {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": "Request param text_1 or text_2 is empty",
                        "error_type": "Input Validation Error"
                    })),
                )
                    .into_response();
            }
        }
        RequestPayload::Classify(p) => {
            if is_empty_value(&p.input) {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": "Request param input is empty",
                        "error_type": "Input Validation Error"
                    })),
                )
                    .into_response();
            }
        }
    }

    // 对 Chat 请求，预处理 messages
    prepare_chat_request(&mut payload);

    let initial_model = payload.get_model().to_string();

    let mut response = app_state
        .dispatcher_service
        .execute(&initial_model, |client_config, model_name| {
            // 每次重试可能针对不同的模型（fallback），因此需要更新 payload 中的 model
            // 同时需要克隆上下文数据以传递给异步块
            let app_state = app_state.clone();
            let headers = headers.clone();
            let addr = addr.clone();

            // Clone payload 并在副本上设置新的模型名称
            let mut current_payload = payload.clone();
            current_payload.set_model(model_name.to_string());
            let client_config = client_config.clone();

            async move {
                dispatch_request(&app_state, &headers, addr, &current_payload, &client_config).await
            }
        })
        .await;

    // 如果响应中包含日志元数据但缺少 request_body（通常发生在所有上游都失败时），
    // 在此处补全 request_body 以便记录日志。
    if let Some(meta) = response.extensions_mut().get_mut::<AccessLogMeta>() {
        if meta.request_body.is_none() {
            let payload_value = serde_json::to_value(&payload)
                .unwrap_or(json!({"error": "failed to serialize payload"}));
            let log_body =
                serde_json::to_string(&truncate_json(&payload_value)).unwrap_or_default();
            meta.request_body = Some(log_body);
        }
    }

    response
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
            Ok(req_err) => {
                let error_text = if req_err.is_timeout() {
                    "Request timed out"
                } else if req_err.is_connect() {
                    "Failed to connect to host"
                } else {
                    "External request failed"
                };
                AppError::InternalServerError(error_text.to_string())
            }
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
    // let mut response_body: Value = response.json().await?;
    // 使用 simd-json 加速大 JSON 解析
    let body_bytes = response.bytes().await?;
    let mut buf = body_bytes.to_vec();
    let mut response_body: Value = simd_json::from_slice(&mut buf).map_err(|e| {
        AppError::InternalServerError(format!(
            "Failed to parse upstream JSON with simd-json: {}",
            e
        ))
    })?;

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
