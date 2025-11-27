use axum::{
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use reqwest::multipart::{Form, Part};
use serde_json::json;
use std::sync::Arc;
use tracing::{error, info};

use crate::{
    client::proxy::{build_and_send_request_multipart, get_api_key},
    client::routing::select_clients_by_weight,
    state::app_state::AppState,
};

pub async fn handle_audio_transcription(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    handle_audio_request(app_state, headers, multipart, "audio/transcriptions").await
}

pub async fn handle_audio_translation(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    handle_audio_request(app_state, headers, multipart, "audio/translations").await
}

// 用于在内存中缓存 Multipart 数据的结构体
struct CachedPart {
    name: String,
    data: Vec<u8>,
    file_name: Option<String>,
    content_type: Option<String>,
}

async fn handle_audio_request(
    app_state: Arc<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
    endpoint_path: &str,
) -> Response {
    let mut model_name = String::new();
    let mut cached_parts: Vec<CachedPart> = Vec::new();

    // 1. 解析 Multipart，提取 model 并缓存所有部分
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        let file_name = field.file_name().map(|s| s.to_string());
        let content_type = field.content_type().map(|s| s.to_string());

        // 读取数据到内存
        let data = match field.bytes().await {
            Ok(bytes) => bytes.to_vec(),
            Err(e) => {
                let err_msg = json!({"error": format!("Failed to read field data: {}", e)});
                return (StatusCode::BAD_REQUEST, Json(err_msg)).into_response();
            }
        };

        if name == "model" {
            model_name = String::from_utf8_lossy(&data).to_string();
        }

        cached_parts.push(CachedPart {
            name,
            data,
            file_name,
            content_type,
        });
    }

    if model_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Missing 'model' field in multipart form data"})),
        )
            .into_response();
    }

    // 2. 路由逻辑
    let mut current_model = model_name.clone();

    loop {
        let config_guard = app_state.config_manager.get_config_guard().await;
        let matching_clients = app_state
            .client_manager
            .find_matching_clients(&config_guard, &current_model)
            .await;
        let matching_clients = select_clients_by_weight(matching_clients);

        if matching_clients.is_empty() {
            error!("No matching clients found for model: {}", current_model);
            let err_msg =
                json!({ "error": format!("The model `{}` does not exist.", current_model) });
            return (StatusCode::NOT_FOUND, Json(err_msg)).into_response();
        }

        let mut fallback_triggered = false;
        for client_config in matching_clients {
            // 3. 为每个客户端重新构建 Form
            let mut form = Form::new();
            for part in &cached_parts {
                let mut req_part = Part::bytes(part.data.clone());
                if let Some(fn_str) = &part.file_name {
                    req_part = req_part.file_name(fn_str.clone());
                }
                if let Some(ct_str) = &part.content_type {
                    req_part = match req_part.mime_str(ct_str) {
                        Ok(p) => p,
                        Err(_) => {
                            let mut p = Part::bytes(part.data.clone());
                            if let Some(fn_str) = &part.file_name {
                                p = p.file_name(fn_str.clone());
                            }
                            p
                        }
                    };
                }
                form = form.part(part.name.clone(), req_part);
            }

            let url = format!(
                "{}/{}",
                client_config.base_url.trim_end_matches('/'),
                endpoint_path
            );
            let api_key = get_api_key(&client_config, &headers);

            // 4. 发送请求
            match build_and_send_request_multipart(&app_state, &api_key, &url, form).await {
                Ok(resp) => {
                    let status = resp.status();
                    let headers = resp.headers().clone();
                    let bytes = match resp.bytes().await {
                        Ok(b) => b,
                        Err(e) => {
                            error!("Failed to read response bytes: {}", e);
                            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read response")
                                .into_response();
                        }
                    };

                    let mut response = bytes.into_response();
                    *response.status_mut() = status;
                    if let Some(ct) = headers.get("content-type") {
                        response.headers_mut().insert("content-type", ct.clone());
                    }

                    return response;
                }
                Err(e) => {
                    error!(
                        "Failed to process audio request with client {}: {:?}",
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

        if !fallback_triggered {
            let err_msg =
                json!({ "error": "All upstream providers failed for the requested model." });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(err_msg)).into_response();
        }
    }
}
