use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::debug;

/// Debug timing anchors for diagnosing gateway-induced latency.
///
/// Records 7 key timestamps across the streaming request lifecycle:
///   T1 - request_received:      Gateway handler starts processing
///   T2 - backend_request_sent:  HTTP POST sent to upstream backend
///   T3 - upstream_headers:      Upstream HTTP response headers received
///   T4 - backend_first_chunk:   First SSE data event parsed from upstream
///   T5 - user_first_chunk:      First SSE event polled by Axum (→ client wire)
///   T6 - backend_stream_done:   [DONE] received from upstream
///   T7 - user_stream_complete:  SSE stream fully consumed (tx dropped)
///
/// Only populated when RUST_LOG=debug. Zero overhead otherwise.
#[derive(Debug, Clone)]
pub struct DebugTiming {
    /// T1: Request fully received by gateway handler
    pub request_received: Instant,
    /// T2: Request sent to upstream backend
    pub backend_request_sent: Option<Instant>,
    /// T3: Upstream response headers received
    pub upstream_headers: Option<Instant>,
    /// T4: First SSE chunk data received from upstream
    pub backend_first_chunk: Option<Instant>,
    /// T5: First SSE event polled by Axum (sent to client wire)
    pub user_first_chunk: Option<Instant>,
    /// T6: [DONE] received from upstream
    pub backend_stream_done: Option<Instant>,
    /// T7: SSE stream fully consumed by client
    pub user_stream_complete: Option<Instant>,
    /// Model name
    pub model: String,
    /// Backend name
    pub backend: String,
}

/// Convenience alias for shared debug timing state.
pub type SharedDebugTiming = Option<Arc<Mutex<DebugTiming>>>;

impl DebugTiming {
    /// Create a new DebugTiming with `request_received` set to now.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            request_received: Instant::now(),
            backend_request_sent: None,
            upstream_headers: None,
            backend_first_chunk: None,
            user_first_chunk: None,
            backend_stream_done: None,
            user_stream_complete: None,
            model: model.into(),
            backend: String::new(),
        }
    }

    /// Set the backend name (known after client resolution).
    pub fn set_backend(&mut self, backend: impl Into<String>) {
        self.backend = backend.into();
    }

    /// Output the full timing breakdown via `tracing::debug!`.
    pub fn log(&self) {
        let total = (Instant::now() - self.request_received).as_secs_f64();

        let gateway_to_backend = self
            .backend_request_sent
            .map(|t| (t - self.request_received).as_secs_f64());

        let upstream_network = self
            .upstream_headers
            .zip(self.backend_request_sent)
            .map(|(h, s)| (h - s).as_secs_f64());

        let backend_ttfb = self
            .backend_first_chunk
            .zip(self.upstream_headers)
            .map(|(f, h)| (f - h).as_secs_f64());

        let stream_processing = self
            .user_first_chunk
            .zip(self.backend_first_chunk)
            .map(|(u, b)| (u - b).as_secs_f64());

        let stream_duration = self
            .backend_stream_done
            .zip(self.backend_first_chunk)
            .map(|(d, f)| (d - f).as_secs_f64());

        let end_to_end = self
            .user_stream_complete
            .map(|t| (t - self.request_received).as_secs_f64());

        debug!(
            target: "debug_timing",
            model = %self.model,
            backend = %self.backend,
            total_elapsed_s = format!("{:.4}", total),
            gateway_overhead_s = format!("{:.4}", gateway_to_backend.unwrap_or(0.0)),
            upstream_network_s = format!("{:.4}", upstream_network.unwrap_or(0.0)),
            backend_ttfb_s = format!("{:.4}", backend_ttfb.unwrap_or(0.0)),
            stream_processing_s = format!("{:.4}", stream_processing.unwrap_or(0.0)),
            stream_duration_s = format!("{:.4}", stream_duration.unwrap_or(0.0)),
            end_to_end_s = format!("{:.4}", end_to_end.unwrap_or(0.0)),
            "Streaming request debug timing breakdown",
        );
    }

    /// Output a simpler timing breakdown for non-streaming requests.
    pub fn log_non_streaming(&self) {
        let total = (Instant::now() - self.request_received).as_secs_f64();

        let gateway_to_backend = self
            .backend_request_sent
            .map(|t| (t - self.request_received).as_secs_f64());

        let upstream_response = self
            .upstream_headers
            .zip(self.backend_request_sent)
            .map(|(h, s)| (h - s).as_secs_f64());

        debug!(
            target: "debug_timing",
            model = %self.model,
            backend = %self.backend,
            total_elapsed_s = format!("{:.4}", total),
            gateway_overhead_s = format!("{:.4}", gateway_to_backend.unwrap_or(0.0)),
            upstream_response_s = format!("{:.4}", upstream_response.unwrap_or(0.0)),
            "Non-streaming request debug timing",
        );
    }
}
