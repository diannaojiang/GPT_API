use crate::config::types::ClientConfig;
// Active request gauge is tracked at response body lifetime.
use crate::state::app_state::AppState;
use reqwest::multipart::Form;
use reqwest::{Client, Response};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

// NOTE: Active in-flight requests are tracked at the HTTP response body layer
// (see `metrics::active_requests`) to properly cover SSE streaming lifetimes.

/// 从配置或请求头中获取 API Key
///
/// 优先级：
/// 1. 客户端配置中的固定 key (`client_config.api_key`)
/// 2. 请求头中的 Authorization Bearer token (OpenAI 风格)
/// 3. 请求头中的 x-api-key token (Anthropic 客户端标准)
pub fn get_api_key(
    client_config: &ClientConfig,
    headers: &axum::http::HeaderMap,
) -> Option<String> {
    if let Some(ref key) = client_config.api_key {
        if !key.is_empty() {
            return Some(key.clone());
        }
    }

    // 尝试从 Authorization header 提取 (OpenAI 风格: "Bearer <key>")
    if let Some(key) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|s| s.replace("Bearer ", ""))
        .filter(|s| !s.is_empty())
    {
        return Some(key);
    }

    // 回退：尝试从 x-api-key header 提取 (Anthropic 客户端标准鉴权方式，值本身即 key，无需前缀处理)
    headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(|s| s.to_string())
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
    _client_config: &ClientConfig,
    api_key: &Option<String>,
    url: &str,
    request_body: &Value,
    is_streaming: bool,
    _endpoint: &str,
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    // 获取 HTTP 客户端 (复用)
    let client: Client = app_state.client_manager.get_client();

    // 构造请求
    let mut request_builder = client.post(url);

    if let Some(key) = api_key {
        request_builder = request_builder.header("Authorization", format!("Bearer {}", key));
    }

    // Active requests is tracked at the HTTP response body layer so that streaming
    // stays "active" for the lifetime of the SSE connection.

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
    _client_config: &ClientConfig,
    api_key: &Option<String>,
    url: &str,
    form: Form,
    is_streaming: bool,
    _endpoint: &str,
    _model: &str,
) -> Result<reqwest::Response, Box<dyn std::error::Error + Send + Sync>> {
    let http_client = app_state.client_manager.get_client();

    let mut request_builder = http_client.post(url);

    if let Some(key) = api_key {
        request_builder = request_builder.header("Authorization", format!("Bearer {}", key));
    }

    // Active requests is tracked at the HTTP response body layer so that streaming
    // stays "active" for the lifetime of the SSE connection.

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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    fn make_headers() -> HeaderMap {
        HeaderMap::new()
    }

    fn make_client_config(api_key: Option<&str>) -> ClientConfig {
        ClientConfig {
            name: "test".to_string(),
            base_url: "http://localhost".to_string(),
            api_key: api_key.map(String::from),
            model_match: crate::config::types::ModelMatch {
                match_type: "exact".to_string(),
                value: vec![],
            },
            priority: None,
            fallback: None,
            special_prefix: None,
            stop: None,
            max_tokens: None,
            extra_body: None,
            thinking_format: None,
        }
    }

    fn with_authorization(headers: &mut HeaderMap, key: &str) {
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", key)).unwrap(),
        );
    }

    fn with_x_api_key(headers: &mut HeaderMap, key: &str) {
        headers.insert("x-api-key", HeaderValue::from_str(key).unwrap());
    }

    #[test]
    fn test_get_api_key_from_client_config() {
        let config = make_client_config(Some("config-key"));
        let mut headers = make_headers();
        with_authorization(&mut headers, "header-key");
        with_x_api_key(&mut headers, "x-api-key-value");

        // Client config key takes precedence over headers
        assert_eq!(
            get_api_key(&config, &headers),
            Some("config-key".to_string())
        );
    }

    #[test]
    fn test_get_api_key_from_authorization_header() {
        let config = make_client_config(None);
        let mut headers = make_headers();
        with_authorization(&mut headers, "sk-xxx");

        // Should strip "Bearer " prefix
        assert_eq!(get_api_key(&config, &headers), Some("sk-xxx".to_string()));
    }

    #[test]
    fn test_get_api_key_from_x_api_key_header() {
        let config = make_client_config(None);
        let mut headers = make_headers();
        with_x_api_key(&mut headers, "sk-yyy");

        // x-api-key does not need prefix stripping
        assert_eq!(get_api_key(&config, &headers), Some("sk-yyy".to_string()));
    }

    #[test]
    fn test_get_api_key_authorization_takes_precedence_over_x_api_key() {
        let config = make_client_config(None);
        let mut headers = make_headers();
        with_authorization(&mut headers, "sk-from-auth");
        with_x_api_key(&mut headers, "sk-from-x-api-key");

        // Authorization header takes precedence
        assert_eq!(
            get_api_key(&config, &headers),
            Some("sk-from-auth".to_string())
        );
    }

    #[test]
    fn test_get_api_key_returns_none_when_no_source() {
        let config = make_client_config(None);
        let headers = make_headers();

        // No api key anywhere
        assert_eq!(get_api_key(&config, &headers), None);
    }
}
