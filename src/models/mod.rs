pub mod requests;

#[derive(Clone)]
pub struct AccessLogMeta {
    pub model: String,
    pub error: Option<String>,
    pub request_body: Option<String>,
}
