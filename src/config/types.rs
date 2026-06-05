use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalancingStrategy {
    /// 基于内容的确定性路由 (Rendezvous Hashing)
    /// 当请求提供 routing_keys 时使用确定性 voting,
    /// 无 routing_keys 时自动回退到 random。
    Deterministic,
    /// 加权随机路由 (Efraimidis-Spirakis 采样)
    Random,
    /// 加权最少连接路由，仅在该策略下读取 ACTIVE_REQUESTS metric
    LeastConnections,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalancingConfig {
    #[serde(default)]
    pub strategy: LoadBalancingStrategy,
}

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
    pub openai_clients: Vec<ClientConfig>,
    #[serde(default)]
    pub load_balancing: LoadBalancingConfig,
}

impl Default for LoadBalancingConfig {
    fn default() -> Self {
        Self {
            strategy: LoadBalancingStrategy::Deterministic,
        }
    }
}

impl Default for LoadBalancingStrategy {
    fn default() -> Self {
        Self::Deterministic
    }
}
