use crate::app_error::AppError;
use crate::config::types::ClientConfig;
use crate::db::records::log_non_streaming_request;
use crate::handlers::utils::truncate_json;
use crate::models::requests::RequestPayload;
use crate::models::AccessLogMeta;
use crate::state::app_state::AppState;
use axum::{
    http::HeaderMap,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use eventsource_stream::Eventsource;
use futures::stream::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::error;

struct ToolCallAccumulator {
    index: u64,
    id: Option<String>,
    r#type: Option<String>,
    name: Option<String>,
    arguments: String,
}

struct StreamAccumulator {
    content: String,
    reasoning_content: String,
    role: String,
    tool_calls: HashMap<u64, ToolCallAccumulator>,
}

impl StreamAccumulator {
    fn new() -> Self {
        Self {
            content: String::new(),
            reasoning_content: String::new(),
            role: "assistant".to_string(),
            tool_calls: HashMap::new(),
        }
    }

    fn update(&mut self, delta: &Value) {
        if let Some(r) = delta.get("role").and_then(|s| s.as_str()) {
            self.role = r.to_string();
        }
        if let Some(c) = delta.get("content").and_then(|s| s.as_str()) {
            self.content.push_str(c);
        }
        if let Some(rc) = delta.get("reasoning_content").and_then(|s| s.as_str()) {
            self.reasoning_content.push_str(rc);
        }
        if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tcs {
                if let Some(index) = tc.get("index").and_then(|i| i.as_u64()) {
                    let acc = self.tool_calls.entry(index).or_insert(ToolCallAccumulator {
                        index,
                        id: None,
                        r#type: None,
                        name: None,
                        arguments: String::new(),
                    });
                    if let Some(id) = tc.get("id").and_then(|s| s.as_str()) {
                        acc.id = Some(id.to_string());
                    }
                    if let Some(t) = tc.get("type").and_then(|s| s.as_str()) {
                        acc.r#type = Some(t.to_string());
                    }
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(|s| s.as_str()) {
                            acc.name = Some(name.to_string());
                        }
                        if let Some(args) = func.get("arguments").and_then(|s| s.as_str()) {
                            acc.arguments.push_str(args);
                        }
                    }
                }
            }
        }
    }

    fn to_message_json(&self) -> Value {
        let mut msg = json!({ "role": self.role });
        if !self.content.is_empty() {
            msg["content"] = json!(self.content);
        }
        if !self.reasoning_content.is_empty() {
            msg["reasoning_content"] = json!(self.reasoning_content);
        }
        if !self.tool_calls.is_empty() {
            let mut tcs: Vec<_> = self.tool_calls.values().collect();
            tcs.sort_by_key(|tc| tc.index);
            let tool_calls_json: Vec<Value> = tcs
                .iter()
                .map(|tc| {
                    json!({
                        "id": tc.id,
                        "type": tc.r#type.clone().unwrap_or_else(|| "function".to_string()),
                        "function": { "name": tc.name, "arguments": tc.arguments }
                    })
                })
                .collect();
            msg["tool_calls"] = json!(tool_calls_json);
        }
        msg
    }
}

async fn stream_logger_task(
    mut rx: mpsc::UnboundedReceiver<Value>,
    app_state: Arc<AppState>,
    headers: HeaderMap,
    payload: RequestPayload,
    request_body: Value,
    client_ip: String,
    is_chat: bool,
) {
    let mut accumulator = StreamAccumulator::new();
    let mut last_chunk: Option<Value> = None;
    let mut first_chunk: Option<Value> = None;
    let mut captured_usage: Option<Value> = None;
    let mut captured_finish_reason: Option<String> = None;

    while let Some(chunk) = rx.recv().await {
        if first_chunk.is_none() {
            first_chunk = Some(chunk.clone());
        }
        last_chunk = Some(chunk.clone());

        // Capture Usage (often in the last chunk)
        if let Some(u) = chunk.get("usage") {
            captured_usage = Some(u.clone());
        }

        if let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) {
            if let Some(choice) = choices.first() {
                // Capture Finish Reason
                if let Some(fr) = choice.get("finish_reason").and_then(|s| s.as_str()) {
                    captured_finish_reason = Some(fr.to_string());
                }

                if is_chat {
                    if let Some(delta) = choice.get("delta") {
                        accumulator.update(delta);
                    }
                } else if let Some(text) = choice.get("text").and_then(|s| s.as_str()) {
                    accumulator.content.push_str(text);
                }
            }
        }
    }

    if let Some(mut final_chunk) = last_chunk {
        // If it's empty (e.g. only one chunk), fallback to first_chunk or mock
        if final_chunk.get("choices").is_none() {
            if let Some(first) = first_chunk {
                final_chunk = first;
            }
        }

        // Ensure 'choices' exists AND is not empty
        let choices_valid = final_chunk
            .get("choices")
            .and_then(|c| c.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);

        if !choices_valid {
            final_chunk["choices"] = json!([{ "index": 0 }]);
        }

        // Inject accumulated content
        if is_chat {
            final_chunk["choices"][0]["message"] = accumulator.to_message_json();
            if let Some(choice) = final_chunk["choices"][0].as_object_mut() {
                choice.remove("delta");
            }
        } else {
            final_chunk["choices"][0]["text"] = json!(accumulator.content);
        }

        // Inject captured Usage and Finish Reason
        if let Some(u) = captured_usage {
            final_chunk["usage"] = u;
        }
        if let Some(fr) = captured_finish_reason {
            if let Some(choice) = final_chunk["choices"][0].as_object_mut() {
                choice.insert("finish_reason".to_string(), json!(fr));
            }
        }

        log_non_streaming_request(
            &app_state,
            &headers,
            &payload,
            &request_body,
            &final_chunk,
            client_ip,
        )
        .await;
    }
}

pub async fn process_streaming_response(
    app_state: Arc<AppState>,
    headers: HeaderMap,
    payload: RequestPayload,
    client_ip: String,
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

    // Setup logger channel
    let (tx, rx) = mpsc::unbounded_channel::<Value>();
    let app_state_clone = app_state.clone();
    let headers_clone = headers.clone();
    let payload_clone = payload.clone();
    let request_body_clone = request_body.clone();
    let client_ip_clone = client_ip.clone();

    tokio::spawn(async move {
        stream_logger_task(
            rx,
            app_state_clone,
            headers_clone,
            payload_clone,
            request_body_clone,
            client_ip_clone,
            is_chat,
        )
        .await;
    });

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

                    // Send clone to logger task
                    // Ignore send errors (e.g. logger task panics/drops)
                    let _ = tx.send(value.clone());

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
