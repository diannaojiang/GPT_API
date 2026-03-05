// Copyright 2024 GPT_API Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Metrics worker module - 独立监控线程，将指标收集与请求处理分离
//!
//! 架构：
//! 请求处理线程 ──(channel)──> 独立监控线程
//!      │                              │
//!      │                              ▼
//!      │                       滑动窗口计算
//!      │                       Prometheus 指标更新
//!      ▼                       
//!   路由转发

use std::sync::Arc;
use tokio::sync::mpsc;

use crate::metrics::prometheus::{
    ACTIVE_REQUESTS, ACTIVE_REQUESTS_10M_MAX, ACTIVE_REQUESTS_1H_MAX, ACTIVE_REQUESTS_1M_MAX,
    LATENCY, LATENCY_10M_MAX, LATENCY_1H_MAX, LATENCY_1M_MAX, REQUESTS_TOTAL, RPS, SUCCESS_RATE,
    SUCCESS_RATE_10M, SUCCESS_RATE_1H, SUCCESS_RATE_1M, TOKENS_TOTAL, TPS, TPS_10M_AVG, TPS_1H_AVG,
    TPS_1M_AVG,
};
use crate::metrics::sliding_window;

/// 指标数据消息 - 从请求处理线程发送到监控线程
#[derive(Debug, Clone)]
pub struct MetricEvent {
    /// 请求端点
    pub endpoint: String,
    /// HTTP 状态码字符串
    pub status: String,
    /// 模型名称
    pub model: String,
    /// 后端名称
    pub backend: String,
    /// 请求耗时（秒）
    pub latency: f64,
    /// 是否成功（2xx/3xx）
    pub is_success: bool,
    /// completion tokens 数量（用于 TPS 计算）
    pub completion_tokens: Option<u64>,
    /// prompt tokens 数量（用于统计）
    pub prompt_tokens: Option<u64>,
    /// 请求总耗时（秒），用于 TPS 计算
    pub elapsed: Option<f64>,
}

// ============================================================================
// Channel 管理
// ============================================================================

/// 全局指标 channel 发送端（Arc 包装以支持多线程共享）
pub type MetricsSender = Arc<mpsc::Sender<MetricEvent>>;

/// 创建指标 channel
pub fn create_metrics_channel() -> (MetricsSender, mpsc::Receiver<MetricEvent>) {
    // 使用较大的缓冲区以应对突发流量
    let (tx, rx) = mpsc::channel(1024);
    (Arc::new(tx), rx)
}

/// 检查 channel 是否还有容量（用于监控）
pub fn channel_has_capacity(sender: &MetricsSender) -> bool {
    // 剩余容量 > 100 视为健康
    sender.capacity() > 100
}

// ============================================================================
// Worker 处理逻辑
// ============================================================================

/// 处理单个指标事件 - 在独立线程中运行
fn process_metric_event(event: MetricEvent) {
    let MetricEvent {
        endpoint,
        status,
        model,
        backend,
        latency,
        is_success,
        completion_tokens,
        prompt_tokens,
        elapsed,
    } = event;

    // 更新 Prometheus 计数器
    REQUESTS_TOTAL
        .with_label_values(&[&endpoint, &status, &model, &backend])
        .inc();

    // 更新延迟直方图
    LATENCY
        .with_label_values(&[&model, &backend])
        .observe(latency);

    // 更新滑动窗口
    sliding_window::update_latency_windows(latency, &model, &backend);
    sliding_window::update_success_overall(is_success);
    sliding_window::update_success_windows(is_success, &model, &backend);

    // 更新历史最大延迟
    LATENCY_1M_MAX
        .with_label_values(&[&model, &backend])
        .set(sliding_window::get_latency_1m_max(&model, &backend));
    LATENCY_10M_MAX
        .with_label_values(&[&model, &backend])
        .set(sliding_window::get_latency_10m_max(&model, &backend));
    LATENCY_1H_MAX
        .with_label_values(&[&model, &backend])
        .set(sliding_window::get_latency_1h_max(&model, &backend));

    // 采样当前活跃请求数并推入滑动窗口
    let current_active = ACTIVE_REQUESTS
        .with_label_values(&[&endpoint, &model, &backend])
        .get();
    sliding_window::update_active_windows(current_active as f64, &model, &backend);

    ACTIVE_REQUESTS_1M_MAX
        .with_label_values(&[&endpoint, &model, &backend])
        .set(sliding_window::get_active_1m_max(&model, &backend) as i64);
    ACTIVE_REQUESTS_10M_MAX
        .with_label_values(&[&endpoint, &model, &backend])
        .set(sliding_window::get_active_10m_max(&model, &backend) as i64);
    ACTIVE_REQUESTS_1H_MAX
        .with_label_values(&[&endpoint, &model, &backend])
        .set(sliding_window::get_active_1h_max(&model, &backend) as i64);

    // 更新成功率
    SUCCESS_RATE_1M
        .with_label_values(&[&endpoint, &model, &backend])
        .set(sliding_window::get_success_1m(&model, &backend));
    SUCCESS_RATE_10M
        .with_label_values(&[&endpoint, &model, &backend])
        .set(sliding_window::get_success_10m(&model, &backend));
    SUCCESS_RATE_1H
        .with_label_values(&[&endpoint, &model, &backend])
        .set(sliding_window::get_success_1h(&model, &backend));

    SUCCESS_RATE
        .with_label_values(&[&endpoint, &model, &backend])
        .set(sliding_window::get_success_overall());

    // 更新 RPS
    if latency > 0.0 {
        RPS.with_label_values(&[&endpoint]).set(1.0 / latency);
    }

    // 处理 TPS 和 Token 统计（仅当有 usage 信息时）
    if let (Some(completion), Some(prompt), Some(elapsed_secs)) =
        (completion_tokens, prompt_tokens, elapsed)
    {
        if completion > 0 && elapsed_secs > 0.0 {
            let tps = completion as f64 / elapsed_secs;
            TPS.with_label_values(&[&model, &backend]).observe(tps);
            sliding_window::update_tps_windows(tps, &model, &backend);

            TPS_1M_AVG
                .with_label_values(&[&model, &backend])
                .set(sliding_window::get_tps_1m_avg(&model, &backend));
            TPS_10M_AVG
                .with_label_values(&[&model, &backend])
                .set(sliding_window::get_tps_10m_avg(&model, &backend));
            TPS_1H_AVG
                .with_label_values(&[&model, &backend])
                .set(sliding_window::get_tps_1h_avg(&model, &backend));
        }

        TOKENS_TOTAL
            .with_label_values(&[&model, "completion"])
            .inc_by(completion as f64);
        TOKENS_TOTAL
            .with_label_values(&[&model, "prompt"])
            .inc_by(prompt as f64);
    }
}

/// 启动 metrics worker - 应该在 tokio runtime 中调用
///
/// 这个函数会持续运行直到 channel 关闭
pub async fn start_metrics_worker(mut rx: mpsc::Receiver<MetricEvent>) {
    tracing::info!("Metrics worker started");

    while let Some(event) = rx.recv().await {
        process_metric_event(event);
    }

    tracing::warn!("Metrics worker stopped - channel closed");
}
