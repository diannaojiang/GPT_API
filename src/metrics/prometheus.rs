use prometheus::{
    exponential_buckets, histogram_opts, register_gauge, register_histogram, register_int_gauge,
    Counter, Gauge, Histogram, IntGauge,
};

pub static REQUESTS_TOTAL: once_cell::sync::Lazy<Counter> = once_cell::sync::Lazy::new(|| {
    Counter::new("gpt_api_requests_total", "Total number of HTTP requests").unwrap()
});

pub static ACTIVE_REQUESTS: once_cell::sync::Lazy<IntGauge> = once_cell::sync::Lazy::new(|| {
    register_int_gauge!("gpt_api_active_requests", "Current active requests").unwrap()
});

pub static ACTIVE_REQUESTS_1M_MAX: once_cell::sync::Lazy<IntGauge> =
    once_cell::sync::Lazy::new(|| {
        register_int_gauge!(
            "gpt_api_active_requests_1m_max",
            "Max active requests in 1 minute"
        )
        .unwrap()
    });

pub static ACTIVE_REQUESTS_10M_MAX: once_cell::sync::Lazy<IntGauge> =
    once_cell::sync::Lazy::new(|| {
        register_int_gauge!(
            "gpt_api_active_requests_10m_max",
            "Max active requests in 10 minutes"
        )
        .unwrap()
    });

pub static ACTIVE_REQUESTS_1H_MAX: once_cell::sync::Lazy<IntGauge> =
    once_cell::sync::Lazy::new(|| {
        register_int_gauge!(
            "gpt_api_active_requests_1h_max",
            "Max active requests in 1 hour"
        )
        .unwrap()
    });

pub static SUCCESS_RATE: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!("gpt_api_success_rate", "Overall success rate").unwrap()
});

pub static SUCCESS_RATE_1M: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!("gpt_api_success_rate_1m", "Success rate in last 1 minute").unwrap()
});

pub static SUCCESS_RATE_10M: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!(
        "gpt_api_success_rate_10m",
        "Success rate in last 10 minutes"
    )
    .unwrap()
});

pub static SUCCESS_RATE_1H: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!("gpt_api_success_rate_1h", "Success rate in last 1 hour").unwrap()
});

pub static TTFT: once_cell::sync::Lazy<Histogram> = once_cell::sync::Lazy::new(|| {
    let opts = histogram_opts!(
        "gpt_api_ttft_seconds",
        "Time to First Token in seconds",
        exponential_buckets(0.01, 2.0, 15).unwrap()
    );
    register_histogram!(opts).unwrap()
});

pub static TTFT_10M_MAX: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!("gpt_api_ttft_10m_max", "Max TTFT in 10 minutes").unwrap()
});

pub static TTFT_1H_MAX: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!("gpt_api_ttft_1h_max", "Max TTFT in 1 hour").unwrap()
});

pub static LATENCY: once_cell::sync::Lazy<Histogram> = once_cell::sync::Lazy::new(|| {
    let opts = histogram_opts!(
        "gpt_api_latency_seconds",
        "End-to-end latency in seconds",
        exponential_buckets(0.01, 2.0, 15).unwrap()
    );
    register_histogram!(opts).unwrap()
});

pub static LATENCY_1M_MAX: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!("gpt_api_latency_1m_max", "Max latency in 1 minute").unwrap()
});

pub static LATENCY_10M_MAX: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!("gpt_api_latency_10m_max", "Max latency in 10 minutes").unwrap()
});

pub static LATENCY_1H_MAX: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!("gpt_api_latency_1h_max", "Max latency in 1 hour").unwrap()
});

pub static TPS: once_cell::sync::Lazy<Histogram> = once_cell::sync::Lazy::new(|| {
    let opts = histogram_opts!(
        "gpt_api_tps",
        "Tokens per second",
        exponential_buckets(1.0, 2.0, 15).unwrap()
    );
    register_histogram!(opts).unwrap()
});

pub static TPS_1M_AVG: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!("gpt_api_tps_1m_avg", "Average TPS in 1 minute").unwrap()
});

pub static TPS_10M_AVG: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!("gpt_api_tps_10m_avg", "Average TPS in 10 minutes").unwrap()
});

pub static TPS_1H_AVG: once_cell::sync::Lazy<Gauge> = once_cell::sync::Lazy::new(|| {
    register_gauge!("gpt_api_tps_1h_avg", "Average TPS in 1 hour").unwrap()
});

pub static RPS: once_cell::sync::Lazy<Gauge> =
    once_cell::sync::Lazy::new(|| register_gauge!("gpt_api_rps", "Requests per second").unwrap());

pub static TOKENS_TOTAL: once_cell::sync::Lazy<Counter> = once_cell::sync::Lazy::new(|| {
    Counter::new("gpt_api_tokens_total", "Total tokens processed").unwrap()
});

pub static BACKEND_UP: once_cell::sync::Lazy<IntGauge> = once_cell::sync::Lazy::new(|| {
    register_int_gauge!("gpt_api_backend_up", "Backend status (1=up, 0=down)").unwrap()
});

pub static FAILOVER_TOTAL: once_cell::sync::Lazy<Counter> = once_cell::sync::Lazy::new(|| {
    Counter::new("gpt_api_failover_total", "Total failover events").unwrap()
});

pub static ERRORS_TOTAL: once_cell::sync::Lazy<Counter> =
    once_cell::sync::Lazy::new(|| Counter::new("gpt_api_errors_total", "Total errors").unwrap());
