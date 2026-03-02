use axum::{body::Body, extract::Request, middleware::Next, response::Response};
use std::time::Instant;

use crate::metrics::prometheus::{
    ACTIVE_REQUESTS, ACTIVE_REQUESTS_10M_MAX, ACTIVE_REQUESTS_1H_MAX, ACTIVE_REQUESTS_1M_MAX,
    LATENCY, LATENCY_10M_MAX, LATENCY_1H_MAX, LATENCY_1M_MAX, REQUESTS_TOTAL, RPS, SUCCESS_RATE,
    SUCCESS_RATE_10M, SUCCESS_RATE_1H, SUCCESS_RATE_1M,
};
use crate::metrics::sliding_window;

pub async fn metrics_middleware(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let endpoint = req.uri().path().to_string();

    ACTIVE_REQUESTS.with_label_values(&[&endpoint]).inc();

    let response = next.run(req).await;

    let elapsed = start.elapsed().as_secs_f64();
    let status = response.status().as_u16();
    let status_str = status.to_string();
    let is_success = status >= 200 && status < 400;

    ACTIVE_REQUESTS.with_label_values(&[&endpoint]).dec();
    REQUESTS_TOTAL
        .with_label_values(&[&endpoint, &status_str])
        .inc();

    // Try to get model/backend from response extensions
    let (model_str, backend_str) = response
        .extensions()
        .get::<crate::models::AccessLogMeta>()
        .map(|meta| (meta.model.as_str(), meta.backend.as_str()))
        .unwrap_or(("unknown", "unknown"));

    LATENCY
        .with_label_values(&[model_str, backend_str])
        .observe(elapsed);

    sliding_window::update_latency_windows(elapsed);
    sliding_window::update_active_windows(
        ACTIVE_REQUESTS.with_label_values(&[&endpoint]).get() as f64
    );
    sliding_window::update_success_windows(is_success);
    sliding_window::update_success_overall(is_success);
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
        .with_label_values(&[&endpoint])
        .set(sliding_window::get_active_1m_max() as i64);
    ACTIVE_REQUESTS_10M_MAX
        .with_label_values(&[&endpoint])
        .set(sliding_window::get_active_10m_max() as i64);
    ACTIVE_REQUESTS_1H_MAX
        .with_label_values(&[&endpoint])
        .set(sliding_window::get_active_1h_max() as i64);

    SUCCESS_RATE_1M
        .with_label_values(&[&endpoint])
        .set(sliding_window::get_success_1m());
    SUCCESS_RATE_10M
        .with_label_values(&[&endpoint])
        .set(sliding_window::get_success_10m());
    SUCCESS_RATE_1H
        .with_label_values(&[&endpoint])
        .set(sliding_window::get_success_1h());

    SUCCESS_RATE
        .with_label_values(&[&endpoint])
        .set(if is_success { 1.0 } else { 0.0 });
    RPS.with_label_values(&[&endpoint]).set(1.0 / elapsed);

    response
}
