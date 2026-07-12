use crate::config::types::{ClientConfig, Config};
use reqwest::Client;
use tokio::sync::RwLockReadGuard;

pub struct ClientManager {
    client: Client,
}

impl Default for ClientManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientManager {
    pub fn new() -> Self {
        let client = Client::builder()
            // TCP 连接建立超时：10秒 (快速失败)
            .connect_timeout(std::time::Duration::from_secs(10))
            // 全局总超时：30分钟 (避免截断长流，但防止永久挂起)
            .timeout(std::time::Duration::from_secs(1800))
            // 空闲连接淘汰须短于上游/中间设备的 keepalive 超时，避免复用已被对端静默关闭的陈旧连接
            // 取 15s 以覆盖常见的云 LB/网关短 keepalive 窗口 (5~30s)，显著降低"取出即死"的陈旧连接
            .pool_idle_timeout(std::time::Duration::from_secs(15))
            .pool_max_idle_per_host(64)
            // TCP keepalive 探测防止 NAT/LB 静默切断长连接
            .tcp_keepalive(std::time::Duration::from_secs(30))
            // 禁用 Nagle 算法，降低 SSE 流式小包延迟
            .tcp_nodelay(true)
            .build()
            .expect("Failed to build reqwest client");
        ClientManager { client }
    }

    pub fn get_client(&self) -> Client {
        self.client.clone()
    }

    // 实现find_matching_clients函数
    pub async fn find_matching_clients<'a>(
        &self,
        config: &RwLockReadGuard<'a, Config>,
        model: &str,
    ) -> Vec<ClientConfig> {
        let matching_clients: Vec<ClientConfig> = config
            .openai_clients
            .iter()
            .filter(|client| match client.model_match.match_type.as_str() {
                "keyword" => client
                    .model_match
                    .value
                    .iter()
                    .any(|keyword| model.contains(keyword)),
                "exact" => client.model_match.value.contains(&model.to_string()),
                _ => false,
            })
            .cloned()
            .collect();

        matching_clients
    }
}
