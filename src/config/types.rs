use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model_match: ModelMatch,
    pub priority: Option<u32>,
    pub fallback: Option<String>,
    pub special_prefix: Option<String>,
    pub stop: Option<Vec<String>>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMatch {
    #[serde(rename = "type")]
    pub match_type: String,
    pub value: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub check_config: Option<CheckConfig>,
    pub openai_clients: Vec<ClientConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub interval: u64,
}
