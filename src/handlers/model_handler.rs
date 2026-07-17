use axum::{extract::State, http::HeaderMap, Json};
use futures::future::join_all;
use reqwest::Client;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

use crate::config::types::{ClientConfig, ModelMatch};
use crate::state::app_state::AppState;

fn model_matches_filter(model_id: &str, model_match: &ModelMatch) -> bool {
    match model_match.match_type.as_str() {
        "keyword" => model_match
            .value
            .iter()
            .any(|keyword| model_id.contains(keyword)),
        "exact" => model_match.value.contains(&model_id.to_string()),
        _ => false,
    }
}

/// 从单个客户端获取模型列表
async fn fetch_models_from_client(
    client_config: &ClientConfig,
    http_client: &Client,
    api_key: &Option<String>,
) -> Option<Value> {
    let url = format!("{}/models", client_config.base_url.trim_end_matches('/'));

    let mut request_builder = http_client.get(&url);

    if let Some(key) = api_key {
        request_builder = request_builder.header("Authorization", format!("Bearer {}", key));
    }

    match request_builder.send().await {
        Ok(response) => {
            if response.status().is_success() {
                (response.json::<Value>().await).ok()
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

/// 获取所有模型的列表
pub async fn list_models(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Value> {
    // 获取当前配置
    let config = app_state.config_manager.get_config().await;

    let http_client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build HTTP client");

    // 创建一个任务列表来并发获取所有客户端的模型
    let mut tasks = Vec::new();

    // 从请求头提取 API key（仅从 header 取，不用 config 中的 api_key）
    let api_key = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.replace("Bearer ", "").trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
        });

    // 为每个客户端创建一个异步任务，所有客户端共用同一个 key
    for client_config in &config.openai_clients {
        let client_config_clone = client_config.clone();
        let http_client_clone = http_client.clone();
        let api_key_clone = api_key.clone();

        let task = tokio::spawn(async move {
            fetch_models_from_client(&client_config_clone, &http_client_clone, &api_key_clone).await
        });

        tasks.push(task);
    }

    // 等待所有任务完成
    let results = join_all(tasks).await;

    // 收集所有成功的响应
    let mut all_models = Vec::new();

    for (result, client_config) in results.iter().zip(config.openai_clients.iter()) {
        if let Ok(Some(models)) = result {
            if let Some(data) = models.get("data").and_then(|d| d.as_array()) {
                for model in data {
                    if let Some(id) = model.get("id").and_then(|id| id.as_str()) {
                        if model_matches_filter(id, &client_config.model_match) {
                            all_models.push(model.clone());
                        }
                    }
                }
            } else if let Some(id) = models.get("id").and_then(|id| id.as_str()) {
                if model_matches_filter(id, &client_config.model_match) {
                    all_models.push(models.clone());
                }
            }
        }
    }

    // 去重：根据模型ID去重
    let mut unique_models = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for model in all_models {
        if let Some(id) = model.get("id").and_then(|id| id.as_str()) {
            if !seen_ids.contains(id) {
                seen_ids.insert(id.to_string());
                unique_models.push(model);
            }
        } else {
            // 如果没有ID字段，直接添加
            unique_models.push(model);
        }
    }

    Json(json!({
        "object": "list",
        "data": unique_models
    }))
}
