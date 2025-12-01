use axum::body::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// Part of a multi-modal message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentPart {
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    String(String),
    Array(Vec<ContentPart>),
}

// --- 1. 抽象请求类型 ---
// 使用 enum 来统一不同类型的请求，这是很好的实践。
#[derive(Debug, Clone)] // Add Debug and Clone traits
pub enum RequestPayload {
    Chat(ChatCompletionRequest),
    Completion(CompletionRequest),
    Embedding(EmbeddingRequest),
    Rerank(RerankRequest),
    Score(ScoreRequest),
    Classify(ClassifyRequest),
}

impl RequestPayload {
    pub fn get_model(&self) -> &str {
        match self {
            RequestPayload::Chat(p) => &p.model,
            RequestPayload::Completion(p) => &p.model,
            RequestPayload::Embedding(p) => &p.model,
            RequestPayload::Rerank(p) => &p.model,
            RequestPayload::Score(p) => &p.model,
            RequestPayload::Classify(p) => &p.model,
        }
    }

    pub fn set_model(&mut self, model_name: String) {
        match self {
            RequestPayload::Chat(p) => p.model = model_name,
            RequestPayload::Completion(p) => p.model = model_name,
            RequestPayload::Embedding(p) => p.model = model_name,
            RequestPayload::Rerank(p) => p.model = model_name,
            RequestPayload::Score(p) => p.model = model_name,
            RequestPayload::Classify(p) => p.model = model_name,
        }
    }

    pub fn is_streaming(&self) -> bool {
        match self {
            RequestPayload::Chat(p) => p.stream.unwrap_or(false),
            RequestPayload::Completion(p) => p.stream.unwrap_or(false),
            // 其他类型暂时不支持流式
            _ => false,
        }
    }
}

// Data structures for requests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: Option<MessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_template_kwargs: Option<Value>,
    // Add other fields as needed
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    // Add other fields as needed
}

// --- New vLLM Compatible Requests ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: Value, // String or Vec<String> or Tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankRequest {
    pub model: String,
    pub query: String,
    pub documents: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_n: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreRequest {
    pub model: String,
    pub text_1: Value, // String or Vec<String>
    pub text_2: Value, // String or Vec<String>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifyRequest {
    pub model: String,
    pub input: Value, // String or Vec<String>
}

// --- Internal Structures for Multipart/Audio Handling ---

#[derive(Debug, Clone)]
pub struct MultipartPart {
    pub name: String,
    pub data: Bytes,
    pub file_name: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AudioRequest {
    pub model: String,
    pub parts: Vec<MultipartPart>,
    pub endpoint: String, // To distinguish between transcriptions and translations internally
}
