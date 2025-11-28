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
            .timeout(std::time::Duration::from_secs(180))
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
