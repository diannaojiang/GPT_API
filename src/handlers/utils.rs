use crate::config::types::ClientConfig;
use crate::models::requests::{Message, MessageContent, RequestPayload};
use regex::Regex;
use serde_json::{json, Value};
use std::net::SocketAddr;

use axum::http::HeaderMap;

const HEADER_X_FORWARDED_FOR: &str = "x-forwarded-for";
const HEADER_X_REAL_IP: &str = "x-real-ip";

/// 从请求头中提取客户端真实 IP
///
/// 尝试顺序：
/// 1. `X-Forwarded-For`: 标准代理头，取第一个 IP
/// 2. `X-Real-IP`: Nginx 等常用头
/// 3. `SocketAddr`: TCP 连接的远端地址
pub fn get_client_ip(headers: &HeaderMap, addr: Option<SocketAddr>) -> String {
    if let Some(xff) = headers.get(HEADER_X_FORWARDED_FOR) {
        if let Ok(xff_str) = xff.to_str() {
            let raw_ip = xff_str.split(',').next().unwrap_or(xff_str).trim();
            return clean_ip(raw_ip);
        }
    }

    if let Some(xri) = headers.get(HEADER_X_REAL_IP) {
        if let Ok(xri_str) = xri.to_str() {
            return clean_ip(xri_str.trim());
        }
    }

    if let Some(addr) = addr {
        return clean_ip(&addr.ip().to_string());
    }

    "unknown".to_string()
}

/// 辅助函数：清洗 IP 地址（移除 IPv4-mapped IPv6 前缀）
fn clean_ip(ip: &str) -> String {
    if let Some(ipv4) = ip.strip_prefix("::ffff:") {
        ipv4.to_string()
    } else {
        ip.to_string()
    }
}

/// 处理消息：清理空白字符和合并连续的用户消息
pub fn process_messages(messages: Vec<Message>) -> Vec<Message> {
    if messages.is_empty() {
        return vec![];
    }

    let mut result: Vec<Message> = Vec::new();

    for mut msg in messages {
        // 1. 清理当前消息内容中的空白字符
        let is_empty = if let Some(content) = &mut msg.content {
            match content {
                MessageContent::String(c) => {
                    let trimmed = c.trim().to_string();
                    *c = trimmed;
                    c.is_empty()
                }
                MessageContent::Array(parts) => {
                    parts.iter_mut().for_each(|part| {
                        if part.r#type == "text" {
                            if let Some(text) = &mut part.text {
                                *text = text.trim().to_string();
                            }
                        }
                    });
                    parts.is_empty()
                }
            }
        } else {
            // 如果 content 为 None，可能是 tool call，视为非空
            false
        };

        // 2. 如果消息内容为空，则跳过
        // 注意：这里有一个隐患，如果 content 是 Some("") 且没有 tool_calls，它会被视为 Empty。
        // 但如果 content 是 None (tool call)，它会被保留。
        if is_empty && msg.tool_calls.is_none() {
            continue;
        }

        // 3. 处理合并逻辑
        if let Some(last_message) = result.last_mut() {
            if last_message.role == "user" && msg.role == "user" {
                *last_message = msg;
            } else {
                result.push(msg);
            }
        } else {
            result.push(msg);
        }
    }
    result
}

/// 过滤空消息
pub fn filter_empty_messages(messages: Vec<Message>) -> Vec<Message> {
    messages
        .into_iter()
        .filter(|message| {
            if let Some(content) = &message.content {
                match content {
                    MessageContent::String(c) => !c.trim().is_empty(),
                    MessageContent::Array(parts) => !parts.is_empty(),
                }
            } else {
                // 如果 content 为 None，只有当 tool_calls 存在时才保留
                message.tool_calls.is_some()
            }
        })
        .collect()
}

use once_cell::sync::Lazy;

static THINK_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"<think>.*?</think>").unwrap());

/// 移除助手消息中的思考标签
pub fn remove_think_tags(messages: Vec<Message>) -> Vec<Message> {
    messages
        .into_iter()
        .map(|mut message| {
            if message.role == "assistant" {
                if let Some(MessageContent::String(content)) = &message.content {
                    let new_content = THINK_TAG_RE.replace_all(content, "").to_string();
                    message.content = Some(MessageContent::String(new_content));
                }
            }
            message
        })
        .collect()
}

/// 合并停止词
pub fn merge_stop_words(
    client_stop: Option<&Vec<String>>,
    request_stop: Option<Vec<String>>,
) -> Option<Vec<String>> {
    match (client_stop, request_stop) {
        (Some(client_stop_words), Some(request_stop_words)) => {
            let mut merged: Vec<String> = client_stop_words.clone();
            for word in request_stop_words {
                if !merged.contains(&word) {
                    merged.push(word);
                }
            }
            Some(merged)
        }
        (Some(client_stop_words), None) => Some(client_stop_words.clone()),
        (None, Some(request_stop_words)) => Some(request_stop_words),
        (None, None) => None,
    }
}

/// 智能调整 max_tokens
pub fn adjust_max_tokens(
    client_max_tokens: Option<u32>,
    request_max_tokens: Option<u32>,
) -> Option<u32> {
    match (client_max_tokens, request_max_tokens) {
        (Some(client_limit), Some(requested)) => {
            if requested > client_limit {
                Some(client_limit)
            } else {
                Some(requested)
            }
        }
        (Some(client_limit), None) => Some(client_limit),
        (None, Some(requested)) => Some(requested),
        (None, None) => None,
    }
}

/// 通用函数：为各种请求类型构建请求体
pub fn build_request_body_generic(
    payload: &RequestPayload,
    client_config: &ClientConfig,
    stream: bool,
) -> Value {
    match payload {
        RequestPayload::Chat(p) => {
            let adjusted_max_tokens = adjust_max_tokens(client_config.max_tokens, p.max_tokens);
            let merged_stop = merge_stop_words(client_config.stop.as_ref(), p.stop.clone());

            let mut body = json!({
                "model": p.model,
                "messages": p.messages,
                "stream": stream,
            });

            if let Some(temp) = p.temperature {
                body["temperature"] = json!(temp);
            }
            if let Some(tokens) = adjusted_max_tokens {
                body["max_tokens"] = json!(tokens);
            }
            if let Some(stop) = merged_stop {
                body["stop"] = json!(stop);
            }
            if let Some(tools) = &p.tools {
                body["tools"] = tools.clone();
            }
            if let Some(kwargs) = &p.chat_template_kwargs {
                body["chat_template_kwargs"] = kwargs.clone();
            }
            body
        }
        RequestPayload::Completion(p) => {
            let adjusted_max_tokens = adjust_max_tokens(client_config.max_tokens, p.max_tokens);
            let merged_stop = merge_stop_words(client_config.stop.as_ref(), p.stop.clone());

            let mut body = json!({
                "model": p.model,
                "prompt": p.prompt,
                "stream": stream,
            });

            if let Some(temp) = p.temperature {
                body["temperature"] = json!(temp);
            }
            if let Some(tokens) = adjusted_max_tokens {
                body["max_tokens"] = json!(tokens);
            }
            if let Some(stop) = merged_stop {
                body["stop"] = json!(stop);
            }
            body
        }
        RequestPayload::Embedding(p) => serde_json::to_value(p).unwrap_or(json!({})),
        RequestPayload::Rerank(p) => serde_json::to_value(p).unwrap_or(json!({})),
        RequestPayload::Score(p) => serde_json::to_value(p).unwrap_or(json!({})),
        RequestPayload::Classify(p) => serde_json::to_value(p).unwrap_or(json!({})),
    }
}

/// 通用函数：为非流式响应的 JSON 体添加特殊前缀
pub fn apply_prefix_to_json(response_body: &mut Value, prefix: &str, is_chat: bool) {
    if prefix.is_empty() {
        return;
    }

    if let Some(choices) = response_body
        .get_mut("choices")
        .and_then(|c| c.as_array_mut())
    {
        for choice in choices {
            let text_node = if is_chat {
                choice.get_mut("message").and_then(|m| m.get_mut("content"))
            } else {
                choice.get_mut("text")
            };

            if let Some(content_val) = text_node {
                if let Some(content_str) = content_val.as_str() {
                    *content_val = json!(format!("{}{}", prefix, content_str));
                }
            }
        }
    }
}

/// 递归截断 JSON 对象，用于日志记录
pub fn truncate_json(value: &Value) -> Value {
    match value {
        Value::String(s) => {
            if s.len() > 500 {
                let mut end = 500;
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                json!(format!("{}...[TRUNCATED]", &s[..end]))
            } else {
                value.clone()
            }
        }
        Value::Array(arr) => {
            if arr.len() > 10 {
                let mut new_arr: Vec<Value> = arr.iter().take(10).map(truncate_json).collect();
                new_arr.push(json!(format!("...[TRUNCATED: {} items]", arr.len())));
                Value::Array(new_arr)
            } else {
                Value::Array(arr.iter().map(truncate_json).collect())
            }
        }
        Value::Object(map) => {
            let new_map = map
                .iter()
                .map(|(k, v)| (k.clone(), truncate_json(v)))
                .collect();
            Value::Object(new_map)
        }
        _ => value.clone(),
    }
}

use crate::models::AccessLogMeta;
use axum::{
    body::Bytes,
    extract::{FromRequest, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::de::DeserializeOwned;

/// 自定义 JSON 提取器，用于拦截反序列化错误并返回标准 JSON 格式的错误响应
pub struct CustomJson<T>(pub T);

#[axum::async_trait]
impl<T, S> FromRequest<S> for CustomJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        // 1. 先读取 Bytes
        let bytes = match Bytes::from_request(req, state).await {
            Ok(b) => b,
            Err(err) => return Err(err.into_response()),
        };

        // 2. 尝试反序列化 (使用 simd-json 加速)
        // simd-json 需要可变 buffer，因此需要转换为 Vec<u8>
        let mut buf = bytes.to_vec();

        match simd_json::from_slice::<T>(&mut buf) {
            Ok(data) => Ok(CustomJson(data)),
            Err(e) => {
                // 3. 失败处理：记录日志元数据
                let error_message = e.to_string();
                // 将 bytes 转换为 string (lossy) 以便记录日志
                // 注意：buf 已经被 from_slice 修改了，但对于打印错误日志来说通常还可以辨认
                let body_str = String::from_utf8_lossy(&bytes).to_string();

                let error_response = json!({
                    "error": format!("Request body validation failed: {}", error_message),
                    "error_type": "InvalidRequest"
                });

                let mut response =
                    (StatusCode::UNPROCESSABLE_ENTITY, Json(error_response)).into_response();

                // 注入 AccessLogMeta 到 Response extensions
                response.extensions_mut().insert(AccessLogMeta {
                    model: "-".to_string(), // 解析失败，无法获知 model
                    error: Some(format!("JSON Error: {}", error_message)),
                    request_body: Some(body_str),
                });

                Err(response)
            }
        }
    }
}
