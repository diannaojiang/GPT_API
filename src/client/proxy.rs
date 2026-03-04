use crate::config::types::ClientConfig;
use crate::metrics::prometheus::ACTIVE_REQUESTS;
use crate::state::app_state::AppState;
use reqwest::multipart::Form;
use reqwest::{Client, Response};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// RAII 守卫：在 drop 时自动递减 ACTIVE_REQUESTS 计数器。
/// 确保无论函数通过正常返回还是通过 `?` 提前返回（超时、连接错误等），
/// 活跃请求计数都会被正确递减。
struct ActiveRequestGuard {
    endpoint: String,
    model: String,
    backend: String,
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        ACTIVE_REQUESTS
            .with_label_values(&[&self.endpoint, &self.model, &self.backend])
            .dec();
    }
}

/// 从配置或请求头中获取 API Key
///
/// 优先级：
/// 1. 客户端配置中的固定 key (`client_config.api_key`)
/// 2. 请求头中的 Authorization Bearer token
pub fn get_api_key(
    client_config: &ClientConfig,
    headers: &axum::http::HeaderMap,
) -> Option<String> {
    if let Some(ref key) = client_config.api_key {
        if !key.is_empty() {
            return Some(key.clone());
        }
    }

    // 尝试从请求头提取
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|s| s.replace("Bearer ", ""))
        .filter(|s| !s.is_empty())
}

/// 构建并发送 HTTP 请求到上游服务
///
/// 该函数负责：
/// 1. 复用 HTTP 客户端连接
/// 2. 构造请求头（包括 API Key）
/// 3. 发送 POST 请求
///
/// 对于流式请求，应用 60秒 TTFB 超时以快速失败
/// 对于非流式请求，仅受 ClientManager 的 1800秒 全局超时限制
pub async fn build_and_send_request(
    app_state: &Arc<AppState>,
    client_config: &ClientConfig,
    api_key: &Option<String>,
    url: &str,
    request_body: &Value,
    is_streaming: bool,
    endpoint: &str,
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    // 获取 HTTP 客户端 (复用)
    let client: Client = app_state.client_manager.get_client();

    // 构造请求
    let mut request_builder = client.post(url);

    if let Some(key) = api_key {
        request_builder = request_builder.header("Authorization", format!("Bearer {}", key));
    }

    let model = request_body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let backend = client_config.name.as_str();

    ACTIVE_REQUESTS
        .with_label_values(&[endpoint, model, backend])
        .inc();
    let _guard = ActiveRequestGuard {
        endpoint: endpoint.to_string(),
        model: model.to_string(),
        backend: backend.to_string(),
    };

    // 根据请求类型应用不同的超时策略
    let response = if is_streaming {
        // 流式请求：设置 60秒 的首字节/响应头超时 (TTFB)
        timeout(
            Duration::from_secs(60),
            request_builder
                .header("Content-Type", "application/json")
                .json(request_body)
                .send(),
        )
        .await
        .map_err(|_| "Upstream service timeout: No response received within 60 seconds")??
    } else {
        // 非流式请求：不设置额外超时，仅依赖 ClientManager 的 1800秒 全局超时
        request_builder
            .header("Content-Type", "application/json")
            .json(request_body)
            .send()
            .await?
    };

    Ok(response)
}

pub async fn build_and_send_request_multipart(
    app_state: &Arc<AppState>,
    client_config: &ClientConfig,
    api_key: &Option<String>,
    url: &str,
    form: Form,
    is_streaming: bool,
    endpoint: &str,
    model: &str,
) -> Result<reqwest::Response, Box<dyn std::error::Error + Send + Sync>> {
    let http_client = app_state.client_manager.get_client();

    let mut request_builder = http_client.post(url);

    if let Some(key) = api_key {
        request_builder = request_builder.header("Authorization", format!("Bearer {}", key));
    }

    let backend = client_config.name.as_str();

    ACTIVE_REQUESTS
        .with_label_values(&[endpoint, model, backend])
        .inc();
    let _guard = ActiveRequestGuard {
        endpoint: endpoint.to_string(),
        model: model.to_string(),
        backend: backend.to_string(),
    };

    // 根据请求类型应用不同的超时策略
    let response = if is_streaming {
        // 流式请求：设置 60秒 的首字节/响应头超时 (TTFB)
        timeout(
            Duration::from_secs(60),
            request_builder.multipart(form).send(),
        )
        .await
        .map_err(|_| "Upstream service timeout: No response received within 60 seconds")??
    } else {
        // 非流式请求：不设置额外超时，仅依赖 ClientManager 的 1800秒 全局超时
        request_builder.multipart(form).send().await?
    };

    Ok(response)
}
