use crate::app_error::AppError;
use crate::client::client_manager::ClientManager;
use crate::client::routing::select_clients;
use crate::config::config_manager::ConfigManager;
use crate::config::types::ClientConfig;
use crate::models::AccessLogMeta;
use axum::response::{IntoResponse, Response};
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, info, warn};

#[derive(Clone)]
pub struct DispatcherService {
    config_manager: Arc<ConfigManager>,
    client_manager: Arc<ClientManager>,
}

impl DispatcherService {
    pub fn new(config_manager: Arc<ConfigManager>, client_manager: Arc<ClientManager>) -> Self {
        Self {
            config_manager,
            client_manager,
        }
    }

    /// 解析给定模型名称对应的客户端列表，并应用负载均衡
    async fn resolve_clients(
        &self,
        model_name: &str,
        routing_keys: &Option<Vec<(String, usize)>>,
    ) -> Result<Vec<ClientConfig>, AppError> {
        let config_guard = self.config_manager.get_config_guard().await;
        let matching_clients = self
            .client_manager
            .find_matching_clients(&config_guard, model_name)
            .await;

        // 应用负载均衡策略（支持加权随机或加权投票）
        let matching_clients = select_clients(matching_clients, routing_keys.clone());

        if matching_clients.is_empty() {
            Err(AppError::ClientNotFound(model_name.to_string()))
        } else {
            Ok(matching_clients)
        }
    }

    pub async fn execute<F, Fut>(
        &self,
        initial_model: &str,
        routing_keys: Option<Vec<(String, usize)>>,
        request_callback: F,
    ) -> Response
    where
        F: FnMut(&ClientConfig, &str) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Response, AppError>> + Send + 'static,
    {
        let cb = Arc::new(Mutex::new(request_callback));
        let mut current_model = initial_model.to_string();
        let mut all_tried_clients = Vec::new();

        loop {
            let clients = match self.resolve_clients(&current_model, &routing_keys).await {
                Ok(c) => c,
                Err(e) => return e.into_response(),
            };

            let execution_result = self
                .execute_client_chain(
                    &clients,
                    &current_model,
                    cb.clone(),
                    &mut all_tried_clients,
                )
                .await;
            // ... (后面保持不变)
            match execution_result {
                // 成功获得响应（包括 4xx 客户端错误，这些被视为业务成功处理）
                Ok(mut response) => {
                    let mut error_msg_opt = None;
                    // 如果响应中包含 AccessLogMeta 且有错误信息，将所有尝试过的客户端追加上去
                    if let Some(meta) = response.extensions_mut().get_mut::<AccessLogMeta>() {
                        if let Some(err_msg) = &mut meta.error {
                            *err_msg = format!("{} (Tried: {:?})", err_msg, all_tried_clients);
                            error_msg_opt = Some(err_msg.clone());
                        }
                    }

                    // 如果是服务端错误 (5xx)，且我们有更新后的错误信息，重新构建响应体以包含 Tried 列表
                    // 这样可以确保客户端收到的错误信息与服务端日志一致
                    if response.status().is_server_error() {
                        if let Some(msg) = error_msg_opt {
                            let new_body = serde_json::json!({
                                "error": msg,
                                "error_type": "internal_error"
                            });
                            let mut new_response =
                                (response.status(), axum::Json(new_body)).into_response();
                            // 必须保留原来的 extension (Meta)，否则日志就丢了
                            *new_response.extensions_mut() = response.extensions().clone();
                            return new_response;
                        }
                    }
                    return response;
                }

                // 触发了 Fallback
                Err(Some(fallback_model)) => {
                    info!(
                        "All clients for model '{}' failed or triggered fallback. Switching to fallback model: '{}'",
                        current_model, fallback_model
                    );
                    current_model = fallback_model;
                    continue;
                }

                // 所有尝试都失败，且没有 Fallback
                Err(None) => {
                    let error_message = format!(
                        "All upstream providers failed for model '{}'. Tried clients: {:?}",
                        current_model, all_tried_clients
                    );
                    warn!("{}", error_message);

                    let mut response =
                        AppError::InternalServerError(error_message.clone()).into_response();

                    // 尝试注入错误日志元数据
                    response.extensions_mut().insert(AccessLogMeta {
                        model: current_model.clone(),
                        error: Some(error_message),
                        request_body: None, // Body logging is handled closer to the request source if needed
                    });
                    return response;
                }
            }
        }
    }

    async fn execute_client_chain<F, Fut>(
        &self,
        clients: &[ClientConfig],
        model_name: &str,
        request_callback: Arc<Mutex<F>>,
        tried_clients_accumulator: &mut Vec<String>,
    ) -> Result<Response, Option<String>>
    where
        F: FnMut(&ClientConfig, &str) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Response, AppError>> + Send + 'static,
    {
        if clients.is_empty() {
            return Err(None);
        }

        let primary_client = &clients[0];
        tried_clients_accumulator.push(primary_client.name.clone());
        debug!("Dispatching request to primary client: {}", primary_client.name);

        let primary_result = {
            let mut cb = request_callback.lock().await;
            cb(primary_client, model_name).await
        };

        match primary_result {
            Ok(mut resp) => {
                let status = resp.status();
                if status.is_success() || status.is_client_error() {
                    if resp.extensions().get::<AccessLogMeta>().is_none() {
                        resp.extensions_mut().insert(AccessLogMeta {
                            model: model_name.to_string(),
                            error: if status.is_client_error() {
                                Some(format!("Upstream client error: {}", status))
                            } else {
                                None
                            },
                            request_body: None,
                        });
                    }
                    return Ok(resp);
                }
                warn!(
                    "Primary client {} failed with status {}. Triggering concurrent fallback...",
                    primary_client.name, status
                );
            }
            Err(e) => {
                warn!(
                    "Primary client {} failed with error: {}. Triggering concurrent fallback...",
                    primary_client.name, e
                );
            }
        }

        let fallback_clients = &clients[1..];
        if fallback_clients.is_empty() {
            if let Some(fallback_model) = &primary_client.fallback {
                return Err(Some(fallback_model.clone()));
            }
            return Err(None);
        }

        debug!(
            "Starting concurrent fallback with {} clients",
            fallback_clients.len()
        );

        let (tx, mut rx) = mpsc::channel::<(usize, Result<Response, AppError>)>(fallback_clients.len());

        for (idx, client_config) in fallback_clients.iter().enumerate() {
            let client = client_config.clone();
            let model = model_name.to_string();
            let cb_clone = request_callback.clone();
            let tx_clone = tx.clone();

            tokio::spawn(async move {
                let result = {
                    let mut cb = cb_clone.lock().await;
                    cb(&client, &model).await
                };
                let _ = tx_clone.send((idx, result)).await;
            });
        }

        drop(tx);

        let mut last_response: Option<Response> = None;
        let mut remaining = fallback_clients.len();

        while let Some((idx, result)) = rx.recv().await {
            remaining -= 1;
            tried_clients_accumulator.push(fallback_clients[idx].name.clone());

            match result {
                Ok(mut resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        debug!("Fallback client {} succeeded", fallback_clients[idx].name);
                        if resp.extensions().get::<AccessLogMeta>().is_none() {
                            resp.extensions_mut().insert(AccessLogMeta {
                                model: model_name.to_string(),
                                error: None,
                                request_body: None,
                            });
                        }
                        return Ok(resp);
                    }
                    if status.is_client_error() {
                        if resp.extensions().get::<AccessLogMeta>().is_none() {
                            resp.extensions_mut().insert(AccessLogMeta {
                                model: model_name.to_string(),
                                error: Some(format!("Upstream client error: {}", status)),
                                request_body: None,
                            });
                        }
                        return Ok(resp);
                    }
                    last_response = Some(resp);
                }
                Err(e) => {
                    warn!(
                        "Fallback client {} failed: {}",
                        fallback_clients[idx].name, e
                    );
                    last_response = Some(e.into_response());
                }
            }

            if remaining == 0 {
                break;
            }
        }

        if let Some(mut resp) = last_response {
            if let Some(meta) = resp.extensions_mut().get_mut::<AccessLogMeta>() {
                meta.model = model_name.to_string();
            } else {
                resp.extensions_mut().insert(AccessLogMeta {
                    model: model_name.to_string(),
                    error: Some("All fallback clients failed".to_string()),
                    request_body: None,
                });
            }
            return Ok(resp);
        }

        if let Some(fallback_model) = &primary_client.fallback {
            return Err(Some(fallback_model.clone()));
        }

        Err(None)
    }
}
