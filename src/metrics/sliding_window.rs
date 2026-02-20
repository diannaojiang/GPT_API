use once_cell::sync::Lazy;
use std::collections::VecDeque;
use std::sync::Mutex;

struct SlidingWindow {
    max_size: usize,
    data: Mutex<VecDeque<f64>>,
}

impl SlidingWindow {
    fn new(max_size: usize) -> Self {
        Self {
            max_size,
            data: Mutex::new(VecDeque::with_capacity(max_size)),
        }
    }

    fn push(&self, value: f64) {
        let mut data = self.data.lock().unwrap();
        if data.len() >= self.max_size {
            data.pop_front();
        }
        data.push_back(value);
    }

    fn max(&self) -> f64 {
        let data = self.data.lock().unwrap();
        data.iter().cloned().fold(0.0 / 0.0, f64::max)
    }

    fn avg(&self) -> f64 {
        let data = self.data.lock().unwrap();
        if data.is_empty() {
            return 0.0;
        }
        data.iter().sum::<f64>() / data.len() as f64
    }

    fn clear(&self) {
        let mut data = self.data.lock().unwrap();
        data.clear();
    }
}

static TTFT_WINDOW_1M: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(60));
static TTFT_WINDOW_10M: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(600));
static TTFT_WINDOW_1H: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(3600));

static LATENCY_WINDOW_1M: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(60));
static LATENCY_WINDOW_10M: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(600));
static LATENCY_WINDOW_1H: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(3600));

static TPS_WINDOW_1M: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(60));
static TPS_WINDOW_10M: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(600));
static TPS_WINDOW_1H: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(3600));

static ACTIVE_WINDOW_1M: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(60));
static ACTIVE_WINDOW_10M: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(600));
static ACTIVE_WINDOW_1H: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(3600));

static SUCCESS_WINDOW_1M: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(60));
static SUCCESS_WINDOW_10M: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(600));
static SUCCESS_WINDOW_1H: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(3600));

pub fn update_ttft_windows(value: f64) {
    TTFT_WINDOW_1M.push(value);
    TTFT_WINDOW_10M.push(value);
    TTFT_WINDOW_1H.push(value);
}

pub fn update_latency_windows(value: f64) {
    LATENCY_WINDOW_1M.push(value);
    LATENCY_WINDOW_10M.push(value);
    LATENCY_WINDOW_1H.push(value);
}

pub fn update_tps_windows(value: f64) {
    TPS_WINDOW_1M.push(value);
    TPS_WINDOW_10M.push(value);
    TPS_WINDOW_1H.push(value);
}

pub fn update_active_windows(value: f64) {
    ACTIVE_WINDOW_1M.push(value);
    ACTIVE_WINDOW_10M.push(value);
    ACTIVE_WINDOW_1H.push(value);
}

pub fn update_success_windows(success: bool) {
    SUCCESS_WINDOW_1M.push(if success { 1.0 } else { 0.0 });
    SUCCESS_WINDOW_10M.push(if success { 1.0 } else { 0.0 });
    SUCCESS_WINDOW_1H.push(if success { 1.0 } else { 0.0 });
}

pub fn get_ttft_1m_max() -> f64 {
    TTFT_WINDOW_1M.max()
}
pub fn get_ttft_10m_max() -> f64 {
    TTFT_WINDOW_10M.max()
}
pub fn get_ttft_1h_max() -> f64 {
    TTFT_WINDOW_1H.max()
}

pub fn get_latency_1m_max() -> f64 {
    LATENCY_WINDOW_1M.max()
}
pub fn get_latency_10m_max() -> f64 {
    LATENCY_WINDOW_10M.max()
}
pub fn get_latency_1h_max() -> f64 {
    LATENCY_WINDOW_1H.max()
}

pub fn get_tps_1m_avg() -> f64 {
    TPS_WINDOW_1M.avg()
}
pub fn get_tps_10m_avg() -> f64 {
    TPS_WINDOW_10M.avg()
}
pub fn get_tps_1h_avg() -> f64 {
    TPS_WINDOW_1H.avg()
}

pub fn get_active_1m_max() -> f64 {
    ACTIVE_WINDOW_1M.max()
}
pub fn get_active_10m_max() -> f64 {
    ACTIVE_WINDOW_10M.max()
}
pub fn get_active_1h_max() -> f64 {
    ACTIVE_WINDOW_1H.max()
}

pub fn get_success_1m() -> f64 {
    SUCCESS_WINDOW_1M.avg()
}
pub fn get_success_10m() -> f64 {
    SUCCESS_WINDOW_10M.avg()
}
pub fn get_success_1h() -> f64 {
    SUCCESS_WINDOW_1H.avg()
}
