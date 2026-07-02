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
    /// JSON 字符串，仅当请求未提供同名字段时注入进请求体。
    pub extra_body: Option<String>,
    /// 每后端覆盖全局 thinking_format；None 时回退全局配置
    #[serde(default)]
    pub thinking_format: Option<ThinkingFormat>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            name: Default::default(),
            base_url: Default::default(),
            api_key: Default::default(),
            model_match: ModelMatch {
                match_type: "exact".to_string(),
                value: Default::default(),
            },
            priority: Default::default(),
            fallback: Default::default(),
            special_prefix: Default::default(),
            stop: Default::default(),
            max_tokens: Default::default(),
            extra_body: Default::default(),
            thinking_format: Default::default(),
        }
    }
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
    #[serde(default)]
    pub thinking_format: Option<ThinkingFormat>,
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

/// 思考内容归一化目标格式。
/// - `Passthrough`（默认）：不做任何转换，原样透传上游思考内容
/// - `ThinkTag`：统一封装为 `<think>...</think>` 包裹在 content 中
/// - `Reasoning`：统一放入 `reasoning` 字段
/// - `ReasoningContent`：统一放入 `reasoning_content` 字段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingFormat {
    Passthrough,
    ThinkTag,
    Reasoning,
    ReasoningContent,
}

impl Default for ThinkingFormat {
    fn default() -> Self {
        Self::Passthrough
    }
}

impl Config {
    /// 解析某后端最终生效的思考格式：优先 per-backend，其次全局，最后 Passthrough。
    pub fn resolve_thinking_format(&self, client: &ClientConfig) -> ThinkingFormat {
        client
            .thinking_format
            .or(self.thinking_format)
            .unwrap_or(ThinkingFormat::Passthrough)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_client_config(thinking_format: Option<ThinkingFormat>) -> ClientConfig {
        ClientConfig {
            name: "test".to_string(),
            base_url: "http://localhost".to_string(),
            api_key: None,
            model_match: ModelMatch {
                match_type: "exact".to_string(),
                value: vec!["test".to_string()],
            },
            priority: None,
            fallback: None,
            special_prefix: None,
            stop: None,
            max_tokens: None,
            extra_body: None,
            thinking_format,
        }
    }

    fn make_config(global_format: Option<ThinkingFormat>) -> Config {
        Config {
            openai_clients: vec![],
            load_balancing: LoadBalancingConfig::default(),
            thinking_format: global_format,
        }
    }

    #[test]
    fn test_resolve_per_backend_overrides_global() {
        let cfg = make_config(Some(ThinkingFormat::ThinkTag));
        let client = make_client_config(Some(ThinkingFormat::Reasoning));
        assert_eq!(
            cfg.resolve_thinking_format(&client),
            ThinkingFormat::Reasoning
        );
    }

    #[test]
    fn test_resolve_global_when_per_backend_none() {
        let cfg = make_config(Some(ThinkingFormat::ReasoningContent));
        let client = make_client_config(None);
        assert_eq!(
            cfg.resolve_thinking_format(&client),
            ThinkingFormat::ReasoningContent
        );
    }

    #[test]
    fn test_resolve_passthrough_when_both_none() {
        let cfg = make_config(None);
        let client = make_client_config(None);
        assert_eq!(
            cfg.resolve_thinking_format(&client),
            ThinkingFormat::Passthrough
        );
    }
}
