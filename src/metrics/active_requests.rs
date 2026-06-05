use axum::body::Body;
use bytes::Bytes;
use http_body::{Frame, SizeHint};
use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::config::types::ClientConfig;
use crate::metrics::prometheus::ACTIVE_REQUESTS;

/// 读取活跃请求计数，按 backend name 聚合。
/// 仅当 strategy 为 LeastConnections 时由 dispatcher 调用。
/// 对于没有 endpoint 的情况（如音频），使用 "unknown" 作为默认值。
pub fn get_active_counts_for_clients(
    clients: &[ClientConfig],
    model_name: &str,
    endpoint: Option<&str>,
) -> HashMap<String, i64> {
    let ep = endpoint.unwrap_or("unknown");
    clients
        .iter()
        .map(|client| {
            let count = ACTIVE_REQUESTS
                .with_label_values(&[ep, model_name, &client.name])
                .get();
            (client.name.clone(), count)
        })
        .collect()
}

pub fn inc_active_requests(endpoint: &str, model: &str, backend: &str) {
    ACTIVE_REQUESTS
        .with_label_values(&[endpoint, model, backend])
        .inc();
}

pub fn dec_active_requests(endpoint: &str, model: &str, backend: &str) {
    ACTIVE_REQUESTS
        .with_label_values(&[endpoint, model, backend])
        .dec();
}

#[derive(Debug, Clone)]
pub struct ActiveRequestLabels {
    pub endpoint: String,
    pub model: String,
    pub backend: String,
}

/// Drop guard that decrements ACTIVE_REQUESTS when dropped.
///
/// IMPORTANT: This must be held by the response body so that it lives
/// for the entire client-facing response lifetime (especially SSE streaming).
#[derive(Debug)]
pub struct ActiveRequestGuard {
    labels: ActiveRequestLabels,
}

impl ActiveRequestGuard {
    pub fn new(labels: ActiveRequestLabels) -> Self {
        inc_active_requests(&labels.endpoint, &labels.model, &labels.backend);
        Self { labels }
    }
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        dec_active_requests(
            &self.labels.endpoint,
            &self.labels.model,
            &self.labels.backend,
        );
    }
}

/// A response body wrapper that holds an `ActiveRequestGuard`.
///
/// When the client disconnects or the body finishes, the body is dropped and
/// the guard decrements the active gauge.
pub struct GuardedBody {
    inner: Body,
    _guard: ActiveRequestGuard,
}

impl GuardedBody {
    pub fn new(inner: Body, labels: ActiveRequestLabels) -> Self {
        Self {
            inner,
            _guard: ActiveRequestGuard::new(labels),
        }
    }
}

impl http_body::Body for GuardedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
