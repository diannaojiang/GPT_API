use crate::app_error::AppError;
use crate::client::client_manager::ClientManager;
use crate::client::routing::select_clients;
use crate::config::config_manager::ConfigManager;
use crate::config::types::ClientConfig;
use crate::models::AccessLogMeta;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::future::Future;
use std::sync::Arc;
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

    /// 执行统一的请求分发逻辑，包含路由、负载均衡和故障转移循环。
    pub async fn execute<F, Fut>(
        &self,
        initial_model: &str,
        routing_keys: Option<Vec<(String, usize)>>,
        mut request_callback: F,
    ) -> Response
    where
        F: FnMut(&ClientConfig, &str) -> Fut + Send,
        Fut: Future<Output = Result<Response, AppError>> + Send,
    {
        let mut current_model = initial_model.to_string();
        let mut all_tried_clients = Vec::new();

        // 主循环：处理故障转移 (Fallback)
        loop {
            // 1. 解析客户端 (传入路由键)
            let clients = match self.resolve_clients(&current_model, &routing_keys).await {
                Ok(c) => c,
                Err(e) => return e.into_response(),
            };

            // 2. 尝试执行客户端链
            let execution_result = self
                .execute_client_chain(
                    &clients,
                    &current_model,
                    &mut request_callback,
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

    /// 遍历客户端列表并执行请求
    async fn execute_client_chain<F, Fut>(
        &self,
        clients: &[ClientConfig],
        model_name: &str,
        request_callback: &mut F,
        tried_clients_accumulator: &mut Vec<String>,
    ) -> Result<Response, Option<String>>
    where
        F: FnMut(&ClientConfig, &str) -> Fut + Send,
        Fut: Future<Output = Result<Response, AppError>> + Send,
    {
        let mut last_response: Option<Response> = None;

        for client_config in clients {
            tried_clients_accumulator.push(client_config.name.clone());
            debug!("Dispatching request to client: {}", client_config.name);

            // 调用回调函数执行实际请求
            let result = request_callback(client_config, model_name).await;

            match result {
                Ok(mut resp) => {
                    let status = resp.status();

                    // 1. 成功 (2xx) -> 直接返回
                    if status.is_success() {
                        // 注入 AccessLogMeta (如果尚未存在)
                        if resp.extensions().get::<AccessLogMeta>().is_none() {
                            resp.extensions_mut().insert(AccessLogMeta {
                                model: model_name.to_string(),
                                error: None,
                                request_body: None,
                            });
                        }
                        return Ok(resp);
                    }

                    // 2. 客户端错误 (4xx) -> 业务错误，不重试，直接返回
                    if status.is_client_error() {
                        // 确保 Log Meta 存在
                        if let Some(meta) = resp.extensions_mut().get_mut::<AccessLogMeta>() {
                            meta.model = model_name.to_string();
                        } else {
                            resp.extensions_mut().insert(AccessLogMeta {
                                model: model_name.to_string(),
                                error: Some(format!("Upstream client error: {}", status)),
                                request_body: None,
                            });
                        }
                        return Ok(resp);
                    }

                    // 3. 服务端错误 (5xx) -> 检查 Fallback
                    if status.is_server_error() {
                        warn!(
                            "Client {} failed with status {}. Checking fallback...",
                            client_config.name, status
                        );

                        if let Some(fallback_model) = &client_config.fallback {
                            // 立即返回 Fallback 模型名称，触发外层循环重启
                            return Err(Some(fallback_model.clone()));
                        }

                        // 如果没有特定 fallback，暂存响应，继续尝试下一个同模型的客户端
                        last_response = Some(resp);
                    }
                }
                Err(e) => {
                    // 网络错误或其他 AppError
                    warn!(
                        "Failed to process request with client {}: {}",
                        client_config.name, e
                    );

                    if let Some(fallback_model) = &client_config.fallback {
                        return Err(Some(fallback_model.clone()));
                    }
                    last_response = Some(e.into_response());
                }
            }
        }

        // 如果循环结束还没有返回 Ok，说明所有客户端都失败了
        if let Some(mut resp) = last_response {
            // 返回最后一个失败的响应
            if let Some(meta) = resp.extensions_mut().get_mut::<AccessLogMeta>() {
                meta.model = model_name.to_string();
            } else {
                resp.extensions_mut().insert(AccessLogMeta {
                    model: model_name.to_string(),
                    error: Some(
                        "All upstream providers failed (forwarding last error)".to_string(),
                    ),
                    request_body: None,
                });
            }
            return Ok(resp);
        }

        // 没有任何响应（例如客户端列表为空，或者逻辑漏洞），返回 None 表示完全失败
        Err(None)
    }
}
