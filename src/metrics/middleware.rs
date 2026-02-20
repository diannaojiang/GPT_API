use axum::{body::Body, extract::Request, middleware::Next, response::Response};
use std::time::Instant;

use crate::metrics::prometheus::{
    ACTIVE_REQUESTS, ACTIVE_REQUESTS_10M_MAX, ACTIVE_REQUESTS_1H_MAX, ACTIVE_REQUESTS_1M_MAX,
    LATENCY, LATENCY_10M_MAX, LATENCY_1H_MAX, LATENCY_1M_MAX, REQUESTS_TOTAL, SUCCESS_RATE_10M,
    SUCCESS_RATE_1H, SUCCESS_RATE_1M,
};
use crate::metrics::sliding_window;

pub async fn metrics_middleware(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();

    ACTIVE_REQUESTS.inc();

    let response = next.run(req).await;

    let elapsed = start.elapsed().as_secs_f64();
    let status = response.status().as_u16();
    let is_success = status >= 200 && status < 400;

    ACTIVE_REQUESTS.dec();
    REQUESTS_TOTAL.inc();

    LATENCY.observe(elapsed);
    sliding_window::update_latency_windows(elapsed);
    sliding_window::update_active_windows(ACTIVE_REQUESTS.get() as f64);
    sliding_window::update_success_windows(is_success);

    LATENCY_1M_MAX.set(sliding_window::get_latency_1m_max());
    LATENCY_10M_MAX.set(sliding_window::get_latency_10m_max());
    LATENCY_1H_MAX.set(sliding_window::get_latency_1h_max());

    ACTIVE_REQUESTS_1M_MAX.set(sliding_window::get_active_1m_max() as i64);
    ACTIVE_REQUESTS_10M_MAX.set(sliding_window::get_active_10m_max() as i64);
    ACTIVE_REQUESTS_1H_MAX.set(sliding_window::get_active_1h_max() as i64);

    SUCCESS_RATE_1M.set(sliding_window::get_success_1m());
    SUCCESS_RATE_10M.set(sliding_window::get_success_10m());
    SUCCESS_RATE_1H.set(sliding_window::get_success_1h());

    response
}
