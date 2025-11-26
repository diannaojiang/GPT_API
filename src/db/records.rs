use chrono::Utc;

use axum::http::HeaderMap;
use serde_json::Value;
use std::sync::Arc;
use tracing::error;

use crate::{
    models::requests::{MessageContent, RequestPayload},
    state::app_state::AppState,
};

/// 数据库记录结构
#[derive(Debug)]
pub struct Record {
    pub time: String,
    pub ip: String,
    pub model: String,
    pub r#type: String,
    pub completion_tokens: i32,
    pub prompt_tokens: i32,
    pub total_tokens: i32,
    pub tool: bool,
    pub multimodal: bool,
    pub headers: String,
    pub request: String,
    pub response: String,
}

/// 记录请求到数据库
pub async fn log_request(app_state: &Arc<AppState>, record: Record) -> Result<(), sqlx::Error> {
    let pool = app_state.db_pool.read().await;
    sqlx::query(
        r#"
        INSERT INTO records (
            Time, IP, Model, Type, CompletionTokens, PromptTokens, TotalTokens,
            tool, multimodal, headers, request, response
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&record.time)
    .bind(&record.ip)
    .bind(&record.model)
    .bind(&record.r#type)
    .bind(record.completion_tokens)
    .bind(record.prompt_tokens)
    .bind(record.total_tokens)
    .bind(record.tool)
    .bind(record.multimodal)
    .bind(&record.headers)
    .bind(&record.request)
    .bind(&record.response)
    .execute(&*pool)
    .await?;

    Ok(())
}

/// 为非流式请求记录日志
pub async fn log_non_streaming_request(
    app_state: &Arc<AppState>,
    headers: &HeaderMap,
    payload: &RequestPayload,
    request_body: &Value,
    response_body: &Value,
) {
    let client_ip = headers
        .get("X-Forwarded-For")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let headers_json = serde_json::to_string(
        &headers
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    serde_json::Value::String(v.to_str().unwrap_or("").to_string()),
                )
            })
            .collect::<serde_json::Map<_, _>>(),
    )
    .unwrap_or_default();

    let usage = response_body.get("usage");
    let prompt_tokens = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    let completion_tokens = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    let total_tokens = usage
        .and_then(|u| u.get("total_tokens"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0);

    let request_type = match payload {
        RequestPayload::Chat(_) => "chat.completions",
        RequestPayload::Completion(_) => "text_completion",
    };

    let tool_used = request_body.get("tools").is_some();

    let is_multimodal = if let RequestPayload::Chat(p) = payload {
        p.messages.iter().any(|m| {
            if let MessageContent::Array(content_array) = &m.content {
                content_array.iter().any(|item| item.r#type == "image_url")
            } else {
                false
            }
        })
    } else {
        false
    };

    let record = Record {
        time: Utc::now().to_rfc3339(),
        ip: client_ip,
        model: payload.get_model().to_string(),
        r#type: request_type.to_string(),
        completion_tokens: completion_tokens.try_into().unwrap_or_default(),
        prompt_tokens: prompt_tokens.try_into().unwrap_or_default(),
        total_tokens: total_tokens.try_into().unwrap_or_default(),
        tool: tool_used,
        multimodal: is_multimodal,
        headers: headers_json,
        request: serde_json::to_string(request_body).unwrap_or_default(),
        response: serde_json::to_string(response_body).unwrap_or_default(),
    };

    if let Err(e) = log_request(app_state, record).await {
        error!("Failed to log request to database: {}", e);
    }
}
