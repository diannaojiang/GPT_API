use crate::app_error::AppError;
use crate::config::types::ClientConfig;
use crate::handlers::utils::truncate_json;
use crate::models::AccessLogMeta;
use axum::{
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use eventsource_stream::Eventsource;
use futures::stream::StreamExt;
use serde_json::{json, Value};
use tracing::error;

pub async fn process_streaming_response(
    response: reqwest::Response,
    client_config: &ClientConfig,
    is_chat: bool,
    request_body: &Value,
) -> Result<Response, AppError> {
    // 如果状态码不成功，直接作为普通 JSON 返回，不要包装成 SSE
    if !response.status().is_success() {
        let status = response.status();
        let body_bytes = response.bytes().await?;

        // 使用 simd-json 解析错误响应
        let mut buf = body_bytes.to_vec();
        let body_json: Value = simd_json::from_slice(&mut buf).unwrap_or_else(|_| {
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
    let stream = response.bytes_stream();
    let special_prefix = client_config.special_prefix.clone().unwrap_or_default();
    let mut prefix_applied = false;

    let content_json_pointer = if is_chat {
        "/choices/0/delta/content"
    } else {
        "/choices/0/text"
    };

    // 使用 eventsource-stream 进行鲁棒的 SSE 解析
    let sse_stream = stream.eventsource().map(move |result| {
        match result {
            Ok(event) => {
                if event.data == "[DONE]" {
                    return Ok(Event::default().data("[DONE]"));
                }

                // 尝试解析 JSON 数据 (使用 simd-json 加速)
                let mut data_str = event.data;
                if let Ok(mut value) = unsafe { simd_json::from_str::<Value>(&mut data_str) } {
                    // 如果配置了 special_prefix，且还没应用过，则尝试注入
                    if !prefix_applied && !special_prefix.is_empty() {
                        if let Some(delta_content) = value.pointer_mut(content_json_pointer) {
                            if let Some(s) = delta_content.as_str() {
                                // 只有当内容不为空时才注入前缀
                                if !s.is_empty() {
                                    *delta_content = json!(format!("{}{}", special_prefix, s));
                                    prefix_applied = true;
                                }
                            }
                        }
                    }
                    // 使用 simd_json 序列化回字符串
                    Ok(Event::default().data(simd_json::to_string(&value).unwrap_or_default()))
                } else {
                    // 如果不是 JSON（或者是其他类型的事件数据），原样转发
                    // 注意：data_str 可能在 from_str 失败时被部分修改，但在 SSE 场景下非 JSON 数据通常是简单的文本，影响不大
                    Ok(Event::default().data(data_str))
                }
            }
            Err(e) => {
                error!("Error parsing SSE stream: {}", e);
                Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            }
        }
    });

    Ok(Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

pub fn extract_error_msg(body: &Value) -> Option<String> {
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
