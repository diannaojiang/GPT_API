use axum::{body::Body, extract::ConnectInfo, http::Request, middleware::Next, response::Response};
use chrono::Local;
use std::net::SocketAddr;
use std::time::Instant;
use tracing::{error, info};

use crate::handlers::utils::get_client_ip;
use crate::models::AccessLogMeta;

/// 访问日志中间件
///
/// 记录详细的请求和响应信息，格式模仿 Nginx Combined Log Format。
/// 并在发生错误时记录请求体以便排查。
pub async fn access_log_middleware(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();

    // 1. 提取请求信息
    let method = req.method().clone();
    let uri = req.uri().clone();
    let version = req.version();
    let headers = req.headers().clone();

    // 提取 IP
    let addr = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0);
    let client_ip = get_client_ip(&headers, addr);

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

    // 4. 提取 Handler 注入的 Model, Error 和 Request Body 信息
    let (model, error_msg, req_body_option) =
        if let Some(meta) = response.extensions().get::<AccessLogMeta>() {
            (
                meta.model.as_str(),
                meta.error.clone().unwrap_or_else(|| "-".to_string()),
                meta.request_body.clone(),
            )
        } else {
            ("-", "-".to_string(), None)
        };

    // 5. 构造 Nginx Combined 风格的日志字符串
    // 格式: IP - - [Time] "Method URI Version" Status Bytes "Referer" "UserAgent" Latency "Model" "ApiKey" "Error" "RequestBody"
    let time_str = Local::now().format("%d/%b/%Y:%H:%M:%S %z");

    let mut log_line = format!(
        "{} - - [{}] \"{} {} {:?}\" {} {} \"-\" \"{}\" {:.3}s \"{}\" \"{}\" {:?}",
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
        // 仅在错误时记录请求体
        // 不再进行中间件层面的截断，完全信任上层逻辑 (handler) 传递的内容
        // 即使上层传递的是原始的长字符串(例如 json 解析失败)，也完整记录
        if let Some(body) = req_body_option {
            log_line.push_str(&format!(" {:?}", body));
        } else {
            log_line.push_str(" \"-\"");
        }
        error!(target: "access_log", "{}", log_line);
    } else {
        // 成功请求不记录请求体
        info!(target: "access_log", "{}", log_line);
    }

    response
}
