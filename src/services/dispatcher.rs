use crate::app_error::AppError;
use crate::client::client_manager::ClientManager;
use crate::client::routing::select_clients_by_weight;
use crate::config::config_manager::ConfigManager;
use crate::config::types::ClientConfig;
use crate::models::AccessLogMeta;
use axum::response::{IntoResponse, Response};
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
    async fn resolve_clients(&self, model_name: &str) -> Result<Vec<ClientConfig>, AppError> {
        let config_guard = self.config_manager.get_config_guard().await;
        let matching_clients = self
            .client_manager
            .find_matching_clients(&config_guard, model_name)
            .await;

        // 应用基于权重的负载均衡策略
        let matching_clients = select_clients_by_weight(matching_clients);

        if matching_clients.is_empty() {
            Err(AppError::ClientNotFound(model_name.to_string()))
        } else {
            Ok(matching_clients)
        }
    }

    /// 执行统一的请求分发逻辑，包含路由、负载均衡和故障转移循环。
    ///
    /// # 参数
    /// - `initial_model`: 初始请求的模型名称
    /// - `request_callback`: 一个异步闭包，接受 `ClientConfig` 和 `current_model_name`，负责构建并发送实际的 HTTP 请求。
    ///
    /// # 返回
    /// - `Response`: 最终的 HTTP 响应（成功或错误）
    pub async fn execute<F, Fut>(&self, initial_model: &str, mut request_callback: F) -> Response
    where
        F: FnMut(&ClientConfig, &str) -> Fut + Send,
        Fut: Future<Output = Result<Response, AppError>> + Send,
    {
        let mut current_model = initial_model.to_string();

        // 主循环：处理故障转移 (Fallback)
        // 当所有客户端都失败且定义了 fallback 模型时，更新 current_model 并重新开始循环
        loop {
            // 1. 解析客户端
            let clients = match self.resolve_clients(&current_model).await {
                Ok(c) => c,
                Err(e) => return e.into_response(),
            };

            let matching_client_names: Vec<String> =
                clients.iter().map(|c| c.name.clone()).collect();

            // 2. 尝试执行客户端链
            // 这个内部循环尝试当前模型对应的所有可用客户端（负载均衡结果）
            let execution_result = self
                .execute_client_chain(&clients, &current_model, &mut request_callback)
                .await;

            match execution_result {
                // 成功获得响应（包括 4xx 客户端错误，这些被视为业务成功处理）
                Ok(response) => return response,

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
                        current_model, matching_client_names
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
    ) -> Result<Response, Option<String>>
    where
        F: FnMut(&ClientConfig, &str) -> Fut + Send,
        Fut: Future<Output = Result<Response, AppError>> + Send,
    {
        let mut last_response: Option<Response> = None;

        for client_config in clients {
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
