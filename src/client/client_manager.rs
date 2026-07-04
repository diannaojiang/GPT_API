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
            // TCP keepalive: 30秒, 防止中间设备静默切断长连接
            .tcp_keepalive(std::time::Duration::from_secs(30))
            // 空闲连接存活时长: 5分钟, 减少高并发下频繁建连开销
            .pool_idle_timeout(std::time::Duration::from_secs(300))
            // 每 host 最大空闲连接数: 64, 适配 HTTP/1.1 高并发
            .pool_max_idle_per_host(64)
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
