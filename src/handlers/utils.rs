use crate::config::types::ClientConfig;
use crate::models::requests::{Message, MessageContent, RequestPayload};
use regex::Regex;
use serde_json::{json, Value};

/// 处理消息：清理空白字符和合并连续的用户消息
pub fn process_messages(messages: Vec<Message>) -> Vec<Message> {
    if messages.is_empty() {
        return vec![];
    }

    let mut result: Vec<Message> = Vec::new();

    for mut msg in messages {
        // 1. 清理当前消息内容中的空白字符
        let is_empty = match &mut msg.content {
            MessageContent::String(content) => {
                let trimmed = content.trim().to_string();
                *content = trimmed;
                content.is_empty()
            }
            MessageContent::Array(parts) => {
                // For multimodal, we consider it non-empty if it has any parts.
                // The actual text part trimming happens here too.
                parts.iter_mut().for_each(|part| {
                    if part.r#type == "text" {
                        if let Some(text) = &mut part.text {
                            *text = text.trim().to_string();
                        }
                    }
                });
                parts.is_empty()
            }
        };

        // 2. 如果消息内容为空，则跳过
        if is_empty {
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
            match &message.content {
                MessageContent::String(content) => !content.trim().is_empty(),
                MessageContent::Array(parts) => !parts.is_empty(), // Keep non-empty multimodal messages
            }
        })
        .collect()
}

/// 移除助手消息中的思考标签
pub fn remove_think_tags(messages: Vec<Message>) -> Vec<Message> {
    let think_tag_re = Regex::new(r"<think>.*?</think>").unwrap();
    messages
        .into_iter()
        .map(|mut message| {
            if message.role == "assistant" {
                if let MessageContent::String(content) = message.content {
                    let new_content = think_tag_re.replace_all(&content, "").to_string();
                    message.content = MessageContent::String(new_content);
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

/// 通用函数：为 `chat` 和 `completion` 请求构建请求体
pub fn build_request_body_generic(
    payload: &RequestPayload,
    client_config: &ClientConfig,
    stream: bool,
) -> Value {
    let (max_tokens_payload, temp_payload, stop_payload) = match payload {
        RequestPayload::Chat(p) => (p.max_tokens, p.temperature, p.stop.clone()),
        RequestPayload::Completion(p) => (p.max_tokens, p.temperature, p.stop.clone()),
    };

    let adjusted_max_tokens = adjust_max_tokens(client_config.max_tokens, max_tokens_payload);
    let merged_stop = merge_stop_words(client_config.stop.as_ref(), stop_payload);

    let mut request_body = json!({
        "model": payload.get_model(),
        "stream": stream,
    });

    // 添加可选参数
    if let Some(temp) = temp_payload {
        request_body["temperature"] = json!(temp);
    }
    if let Some(tokens) = adjusted_max_tokens {
        request_body["max_tokens"] = json!(tokens);
    }
    if let Some(stop) = merged_stop {
        request_body["stop"] = json!(stop);
    }

    // 添加特定于请求类型的参数
    match payload {
        RequestPayload::Chat(p) => {
            request_body["messages"] = json!(&p.messages);
            if let Some(tools) = &p.tools {
                request_body["tools"] = tools.clone();
            }
        }
        RequestPayload::Completion(p) => {
            request_body["prompt"] = json!(&p.prompt);
        }
    }

    request_body
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
