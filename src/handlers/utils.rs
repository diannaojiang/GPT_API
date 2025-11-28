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

/// 移除助手消息中的思考标签
pub fn remove_think_tags(messages: Vec<Message>) -> Vec<Message> {
    let think_tag_re = Regex::new(r"<think>.*?</think>").unwrap();
    messages
        .into_iter()
        .map(|mut message| {
            if message.role == "assistant" {
                if let Some(MessageContent::String(content)) = &message.content {
                    let new_content = think_tag_re.replace_all(content, "").to_string();
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
