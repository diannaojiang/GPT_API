use axum::http::HeaderMap;
use reqwest::multipart::Form;
use serde_json::Value;
use std::sync::Arc;
use tracing;

use crate::{config::types::ClientConfig, state::app_state::AppState};

/// 从配置或传递的 Header 中获取 API Key
pub fn get_api_key<'a>(client_config: &ClientConfig, headers: &HeaderMap) -> String {
    // 首先检查客户端配置中是否有 API Key
    if let Some(key) = &client_config.api_key {
        if !key.is_empty() {
            // Add this check
            return key.clone();
        }
    }

    // 如果配置中没有，则从请求头中获取
    if let Some(auth_header) = headers.get("Authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(key) = auth_str.strip_prefix("Bearer ") {
                return key.to_string();
            } else {
                return auth_str.to_string();
            }
        }
    }

    // 如果都没有，则返回空字符串
    "".to_string()
}

/// 建立并发送请求的通用函式
pub async fn build_and_send_request(
    app_state: &Arc<AppState>,
    _client_config: &ClientConfig,
    api_key: &str,
    url: &str,
    request_body: &Value,
) -> Result<reqwest::Response, Box<dyn std::error::Error + Send + Sync>> {
    let http_client = app_state.client_manager.get_client();

    let response = http_client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(request_body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        tracing::error!(
            "Backend request failed with status: {}, body: {}",
            status,
            error_body
        );
        return Err(format!("Backend request failed with status: {}", status).into());
    }

    Ok(response)
}

pub async fn build_and_send_request_multipart(
    app_state: &Arc<AppState>,
    api_key: &str,
    url: &str,
    form: Form,
) -> Result<reqwest::Response, Box<dyn std::error::Error + Send + Sync>> {
    let http_client = app_state.client_manager.get_client();

    let response = http_client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        tracing::error!(
            "Backend request failed with status: {}, body: {}",
            status,
            error_body
        );
        return Err(format!("Backend request failed with status: {}", status).into());
    }

    Ok(response)
}
