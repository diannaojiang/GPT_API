use axum::{extract::State, http::HeaderMap, Json};
use futures::future::join_all;
use reqwest::Client;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

use crate::config::types::{ClientConfig, ModelMatch};
use crate::services::models_cache::{CacheKey, CacheOutcome, ModelsCache};
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

/// Aggregate, filter, and deduplicate models across every configured client.
/// Mirrors the original implementation verbatim so the external JSON contract
/// (`{"object":"list","data":[...]}`) is unchanged.
async fn aggregate_models(
    openai_clients: &[ClientConfig],
    http_client: &Client,
    api_key: Option<String>,
) -> Value {
    let mut tasks = Vec::new();
    for client_config in openai_clients {
        let client_config_clone = client_config.clone();
        let http_client_clone = http_client.clone();
        let api_key_clone = api_key.clone();

        let task = tokio::spawn(async move {
            fetch_models_from_client(&client_config_clone, &http_client_clone, &api_key_clone).await
        });
        tasks.push(task);
    }
    let results = join_all(tasks).await;

    let mut all_models = Vec::new();
    for (result, client_config) in results.iter().zip(openai_clients.iter()) {
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

    let mut unique_models = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    for model in all_models {
        if let Some(id) = model.get("id").and_then(|id| id.as_str()) {
            if !seen_ids.contains(id) {
                seen_ids.insert(id.to_string());
                unique_models.push(model);
            }
        } else {
            unique_models.push(model);
        }
    }

    json!({
        "object": "list",
        "data": unique_models
    })
}

fn extract_credential_bytes(headers: &HeaderMap) -> Option<Vec<u8>> {
    if let Some(value) = headers.get("authorization") {
        if let Ok(s) = value.to_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                let body = trimmed
                    .strip_prefix("Bearer ")
                    .or_else(|| trimmed.strip_prefix("bearer "))
                    .unwrap_or(trimmed);
                let body = body.trim();
                if !body.is_empty() {
                    return Some(body.as_bytes().to_vec());
                }
            }
        }
    }
    if let Some(value) = headers.get("x-api-key") {
        if let Ok(s) = value.to_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.as_bytes().to_vec());
            }
        }
    }
    None
}

fn make_key(cache: &ModelsCache, generation: u64, credential: Option<&[u8]>) -> CacheKey {
    CacheKey {
        generation,
        credential: cache.fingerprint(credential),
    }
}

pub async fn list_models(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<Value> {
    let (config, generation) = app_state.config_manager.get_config_with_generation().await;
    let ttl_seconds = config.models_cache.ttl_seconds;
    let credential_bytes = extract_credential_bytes(&headers);

    if ttl_seconds == 0 {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to build HTTP client");
        let key_str = credential_bytes.and_then(|b| String::from_utf8(b).ok());
        return Json(aggregate_models(&config.openai_clients, &http_client, key_str).await);
    }

    let key = make_key(
        &app_state.models_cache,
        generation,
        credential_bytes.as_deref(),
    );
    match app_state.models_cache.lookup(key) {
        CacheOutcome::Fresh(value) => return Json(value),
        CacheOutcome::Empty => {}
    }

    let (is_leader, inflight) = app_state.models_cache.begin_refresh(key);
    if !is_leader {
        app_state.models_cache.wait_for_leader(key, &inflight).await;
        match app_state.models_cache.lookup(key) {
            CacheOutcome::Fresh(value) => return Json(value),
            CacheOutcome::Empty => {
                if let Some(previous) = app_state.models_cache.previous_value(key) {
                    return Json(previous);
                }
            }
        }
    }

    let http_client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build HTTP client");
    let key_str = credential_bytes.and_then(|b| String::from_utf8(b).ok());
    let aggregated = aggregate_models(&config.openai_clients, &http_client, key_str).await;

    app_state
        .models_cache
        .insert(key, aggregated.clone(), Duration::from_secs(ttl_seconds));
    app_state.models_cache.end_refresh(key, &inflight);
    Json(aggregated)
}
