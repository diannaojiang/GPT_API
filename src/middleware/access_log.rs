use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use chrono::Utc;
use std::time::Instant;
use tracing::{error, info};

use crate::handlers::utils::get_client_ip;

// 用于在 Handler 和 Middleware 之间传递元数据的结构体
#[derive(Clone)]
pub struct AccessLogMeta {
    pub model: String,
    pub error: Option<String>,
}

pub async fn access_log_middleware(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();

    // 1. 提取请求信息
    let method = req.method().clone();
    let uri = req.uri().clone();
    let version = req.version();
    let headers = req.headers().clone();

    // 提取 IP
    let client_ip = get_client_ip(&headers);

    // 提取 User-Agent
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-");

    // 提取 API Key (简单脱敏，只取前几位，或者如果太短就全取)
    let api_key = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.strip_prefix("Bearer ").unwrap_or(s))
        .map(|s| {
            if s.len() > 10 {
                format!("{}...", &s[..8])
            } else {
                s.to_string()
            }
        })
        .unwrap_or_else(|| "-".to_string());

    // 2. 执行后续处理
    let response = next.run(req).await;

    // 3. 提取响应信息
    let latency = start.elapsed();
    let status = response.status();

    // 尝试获取 Content-Length (并不总是存在，尤其是流式响应)
    let body_bytes = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-");

    // 4. 提取 Handler 注入的 Model 和 Error 信息
    let (model, error_msg) = if let Some(meta) = response.extensions().get::<AccessLogMeta>() {
        (
            meta.model.as_str(),
            meta.error.clone().unwrap_or_else(|| "-".to_string()),
        )
    } else {
        ("-", "-".to_string())
    };

    // 5. 构造 Nginx Combined 风格的日志字符串
    // 格式: IP - - [Time] "Method URI Version" Status Bytes "Referer" "UserAgent" Latency "Model" "ApiKey" "Error"
    let time_str = Utc::now().format("%d/%b/%Y:%H:%M:%S %z");

    let log_line = format!(
        "{} - - [{}] \"{} {} {:?}\" {} {} \"-\" \"{}\" {:.3}s \"{}\" \"{}\" \"{}\"",
        client_ip,
        time_str,
        method,
        uri,
        version,
        status.as_u16(),
        body_bytes,
        user_agent,
        latency.as_secs_f64(),
        model,
        api_key,
        error_msg
    );

    // 6. 根据状态码决定日志级别
    if status.is_server_error() || status.is_client_error() {
        error!("{}", log_line);
    } else {
        info!("{}", log_line);
    }

    response
}
