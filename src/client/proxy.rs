use crate::config::types::ClientConfig;
use crate::state::app_state::AppState;
use reqwest::multipart::Form;
use reqwest::{Client, Response};
use serde_json::Value;
use std::sync::Arc;

/// 从配置或请求头中获取 API Key
///
/// 优先级：
/// 1. 客户端配置中的固定 key (`client_config.api_key`)
/// 2. 请求头中的 Authorization Bearer token
pub fn get_api_key(client_config: &ClientConfig, headers: &axum::http::HeaderMap) -> String {
    if let Some(ref key) = client_config.api_key {
        if !key.is_empty() {
            return key.clone();
        }
    }

    // 尝试从请求头提取
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|s| s.replace("Bearer ", ""))
        .unwrap_or_default()
}

/// 构建并发送 HTTP 请求到上游服务
///
/// 该函数负责：
/// 1. 复用 HTTP 客户端连接
/// 2. 构造请求头（包括 API Key）
/// 3. 发送 POST 请求
pub async fn build_and_send_request(
    app_state: &Arc<AppState>,
    _client_config: &ClientConfig,
    api_key: &str,
    url: &str,
    request_body: &Value,
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    // 获取 HTTP 客户端 (复用)
    let client: Client = app_state.client_manager.get_client();

    // 构造并发送请求
    // 注意：ClientManager 默认设置了 180s 超时
    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(request_body)
        .send()
        .await?;

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

    Ok(response)
}
