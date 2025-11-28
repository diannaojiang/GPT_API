use axum::{extract::State, Json};
use futures::future::join_all;
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::config::types::ClientConfig;
use crate::state::app_state::AppState;

/// 从单个客户端获取模型列表
async fn fetch_models_from_client(
    client_config: &ClientConfig,
    http_client: &Client,
) -> Option<Value> {
    let url = format!("{}/models", client_config.base_url.trim_end_matches('/'));

    match http_client.get(&url).send().await {
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
pub async fn list_models(State(app_state): State<Arc<AppState>>) -> Json<Value> {
    // 获取当前配置
    let config = app_state.config_manager.get_config().await;

    // 获取HTTP客户端
    let http_client = Client::new();

    // 创建一个任务列表来并发获取所有客户端的模型
    let mut tasks = Vec::new();

    // 为每个客户端创建一个异步任务
    for client_config in &config.openai_clients {
        let client_config_clone = client_config.clone();
        let http_client_clone = http_client.clone();

        let task = tokio::spawn(async move {
            fetch_models_from_client(&client_config_clone, &http_client_clone).await
        });

        tasks.push(task);
    }

    // 等待所有任务完成
    let results = join_all(tasks).await;

    // 收集所有成功的响应
    let mut all_models = Vec::new();

    for result in results {
        if let Ok(Some(models)) = result {
            // 如果响应包含"data"数组，将其展开并添加到all_models
            if let Some(data) = models.get("data").and_then(|d| d.as_array()) {
                for model in data {
                    all_models.push(model.clone());
                }
            } else {
                // 如果响应是单个模型对象，直接添加
                all_models.push(models);
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
