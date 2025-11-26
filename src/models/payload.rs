use super::requests::{
    AudioRequest, ChatCompletionRequest, ClassifyRequest, CompletionRequest, EmbeddingRequest,
    RerankRequest, ScoreRequest,
};
use axum::Json;

#[derive(Clone)]
pub enum RequestPayload {
    Chat(Json<ChatCompletionRequest>),
    Completion(Json<CompletionRequest>),
    Embedding(Json<EmbeddingRequest>),
    Rerank(Json<RerankRequest>),
    Score(Json<ScoreRequest>),
    Classify(Json<ClassifyRequest>),
    Audio(AudioRequest),
}

impl RequestPayload {
    pub fn get_model(&self) -> &str {
        match self {
            RequestPayload::Chat(Json(p)) => &p.model,
            RequestPayload::Completion(Json(p)) => &p.model,
            RequestPayload::Embedding(Json(p)) => &p.model,
            RequestPayload::Rerank(Json(p)) => &p.model,
            RequestPayload::Score(Json(p)) => &p.model,
            RequestPayload::Classify(Json(p)) => &p.model,
            RequestPayload::Audio(p) => &p.model,
        }
    }

    pub fn set_model(&mut self, model_name: String) {
        match self {
            RequestPayload::Chat(Json(p)) => p.model = model_name,
            RequestPayload::Completion(Json(p)) => p.model = model_name,
            RequestPayload::Embedding(Json(p)) => p.model = model_name,
            RequestPayload::Rerank(Json(p)) => p.model = model_name,
            RequestPayload::Score(Json(p)) => p.model = model_name,
            RequestPayload::Classify(Json(p)) => p.model = model_name,
            RequestPayload::Audio(p) => p.model = model_name,
        }
    }

    pub fn is_streaming(&self) -> bool {
        match self {
            RequestPayload::Chat(Json(p)) => p.stream.unwrap_or(false),
            RequestPayload::Completion(Json(p)) => p.stream.unwrap_or(false),
            _ => false, // Embeddings, Rerank, Audio, etc. usually don't stream in the same SSE way
        }
    }
}
