use axum::{body::Body, extract::Request, middleware::Next, response::Response};
use std::time::Instant;

use crate::metrics::prometheus::{
    ACTIVE_REQUESTS, ACTIVE_REQUESTS_10M_MAX, ACTIVE_REQUESTS_1H_MAX, ACTIVE_REQUESTS_1M_MAX,
    LATENCY, LATENCY_10M_MAX, LATENCY_1H_MAX, LATENCY_1M_MAX, REQUESTS_TOTAL, RPS, SUCCESS_RATE,
    SUCCESS_RATE_10M, SUCCESS_RATE_1H, SUCCESS_RATE_1M,
};
use crate::metrics::sliding_window;

/// Endpoints that should be excluded from metrics
const METRICS_SKIP_ENDPOINTS: &[&str] = &["/metrics", "/health", "/v1/models"];

fn should_skip_metrics(endpoint: &str) -> bool {
    METRICS_SKIP_ENDPOINTS.iter().any(|&skip| endpoint == skip)
}

/// Helper to extract model from request body
fn extract_model_from_request(req: &Request<Body>) -> String {
    // Try to get model from query params first
    if let Some(query) = req.uri().query() {
        for param in query.split('&') {
            if param.starts_with("model=") {
                return param[6..].to_string();
            }
        }
    }
    // Default to unknown - actual model will be set from response extensions
    "unknown".to_string()
}

pub async fn metrics_middleware(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let endpoint = req.uri().path().to_string();

    // Skip metrics for non-API endpoints
    if should_skip_metrics(&endpoint) {
        return next.run(req).await;
    }

    // Extract model from request at the start (may be updated later from response)
    let initial_model = extract_model_from_request(&req);
    let pending_backend = "pending";

    // Skip: don't count pending requests - only count when we know the actual backend
    // ACTIVE_REQUESTS.with_label_values(&[&endpoint, &initial_model, pending_backend]).inc();

    let response = next.run(req).await;

    let elapsed = start.elapsed().as_secs_f64();
    let status = response.status().as_u16();
    let status_str = status.to_string();
    let is_success = status >= 200 && status < 400;

    // Get actual model/backend from response extensions
    let access_log_meta = response.extensions().get::<crate::models::AccessLogMeta>();
    let (model_str, backend_str) = access_log_meta
        .map(|meta| {
            // Debug log
            tracing::info!(
                "AccessLogMeta found: model={}, backend={}",
                meta.model,
                meta.backend
            );
            (meta.model.as_str(), meta.backend.as_str())
        })
        .unwrap_or_else(|| {
            tracing::warn!(
                "AccessLogMeta NOT found! endpoint={}, initial_model={}",
                endpoint,
                initial_model
            );
            (&initial_model, pending_backend)
        });

    // If we have AccessLogMeta, switch from pending to actual backend
    // This tracks requests during the time between routing decision and response completion
    let has_access_log = access_log_meta.is_some();
    if has_access_log {
        // Increment actual backend counter
        ACTIVE_REQUESTS
            .with_label_values(&[&endpoint, model_str, backend_str])
            .inc();
    }

    // Decrement actual backend counter to complete the tracking
    if has_access_log {
        // Get count BEFORE decrement to capture peak
        let count_before_dec = ACTIVE_REQUESTS
            .with_label_values(&[&endpoint, model_str, backend_str])
            .get();

        // Update sliding window max tracking
        if count_before_dec > 0 {
            sliding_window::update_active_windows(count_before_dec as f64);
        }

        // Now decrement
        ACTIVE_REQUESTS
            .with_label_values(&[&endpoint, model_str, backend_str])
            .dec();
    }

    REQUESTS_TOTAL
        .with_label_values(&[&endpoint, &status_str, model_str, backend_str])
        .inc();

    LATENCY
        .with_label_values(&[model_str, backend_str])
        .observe(elapsed);

    sliding_window::update_latency_windows(elapsed);
    sliding_window::update_success_overall(is_success);

    // Update success windows for per-backend success rate
    sliding_window::update_success_windows(is_success);
    LATENCY_1M_MAX
        .with_label_values(&[model_str, backend_str])
        .set(sliding_window::get_latency_1m_max());
    LATENCY_10M_MAX
        .with_label_values(&[model_str, backend_str])
        .set(sliding_window::get_latency_10m_max());
    LATENCY_1H_MAX
        .with_label_values(&[model_str, backend_str])
        .set(sliding_window::get_latency_1h_max());

    ACTIVE_REQUESTS_1M_MAX
        .with_label_values(&[&endpoint, model_str, backend_str])
        .set(sliding_window::get_active_1m_max() as i64);
    ACTIVE_REQUESTS_10M_MAX
        .with_label_values(&[&endpoint, model_str, backend_str])
        .set(sliding_window::get_active_10m_max() as i64);
    ACTIVE_REQUESTS_1H_MAX
        .with_label_values(&[&endpoint, model_str, backend_str])
        .set(sliding_window::get_active_1h_max() as i64);

    SUCCESS_RATE_1M
        .with_label_values(&[&endpoint, model_str, backend_str])
        .set(sliding_window::get_success_1m());
    SUCCESS_RATE_10M
        .with_label_values(&[&endpoint, model_str, backend_str])
        .set(sliding_window::get_success_10m());
    SUCCESS_RATE_1H
        .with_label_values(&[&endpoint, model_str, backend_str])
        .set(sliding_window::get_success_1h());

    SUCCESS_RATE
        .with_label_values(&[&endpoint, model_str, backend_str])
        .set(if is_success { 1.0 } else { 0.0 });
    RPS.with_label_values(&[&endpoint]).set(1.0 / elapsed);

    response
}
