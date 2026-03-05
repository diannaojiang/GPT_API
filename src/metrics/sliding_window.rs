use dashmap::DashMap;
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
        data.iter().cloned().fold(f64::NAN, f64::max)
    }

    fn avg(&self) -> f64 {
        let data = self.data.lock().unwrap();
        if data.is_empty() {
            return 0.0;
        }
        data.iter().sum::<f64>() / data.len() as f64
    }
}

type WindowMap = DashMap<String, SlidingWindow>;

static TTFT_WINDOWS_1M: Lazy<WindowMap> = Lazy::new(|| DashMap::new());
static TTFT_WINDOWS_10M: Lazy<WindowMap> = Lazy::new(|| DashMap::new());
static TTFT_WINDOWS_1H: Lazy<WindowMap> = Lazy::new(|| DashMap::new());

static LATENCY_WINDOWS_1M: Lazy<WindowMap> = Lazy::new(|| DashMap::new());
static LATENCY_WINDOWS_10M: Lazy<WindowMap> = Lazy::new(|| DashMap::new());
static LATENCY_WINDOWS_1H: Lazy<WindowMap> = Lazy::new(|| DashMap::new());

static TPS_WINDOWS_1M: Lazy<WindowMap> = Lazy::new(|| DashMap::new());
static TPS_WINDOWS_10M: Lazy<WindowMap> = Lazy::new(|| DashMap::new());
static TPS_WINDOWS_1H: Lazy<WindowMap> = Lazy::new(|| DashMap::new());

static ACTIVE_WINDOWS_1M: Lazy<WindowMap> = Lazy::new(|| DashMap::new());
static ACTIVE_WINDOWS_10M: Lazy<WindowMap> = Lazy::new(|| DashMap::new());
static ACTIVE_WINDOWS_1H: Lazy<WindowMap> = Lazy::new(|| DashMap::new());

static SUCCESS_WINDOWS_1M: Lazy<WindowMap> = Lazy::new(|| DashMap::new());
static SUCCESS_WINDOWS_10M: Lazy<WindowMap> = Lazy::new(|| DashMap::new());
static SUCCESS_WINDOWS_1H: Lazy<WindowMap> = Lazy::new(|| DashMap::new());

static SUCCESS_WINDOW_OVERALL: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(3600));

fn make_key(model: &str, backend: &str) -> String {
    format!("{}:{}", model, backend)
}

fn get_or_create_window(map: &WindowMap, key: &str, max_size: usize) {
    if !map.contains_key(key) {
        map.entry(key.to_string())
            .or_insert_with(|| SlidingWindow::new(max_size));
    }
}

fn get_window_ref<'a>(
    map: &'a WindowMap,
    key: &str,
) -> dashmap::mapref::one::Ref<'a, String, SlidingWindow> {
    map.get(key).unwrap()
}

pub fn update_ttft_windows(value: f64, model: &str, backend: &str) {
    let key = make_key(model, backend);
    get_or_create_window(&TTFT_WINDOWS_1M, &key, 60);
    get_window_ref(&TTFT_WINDOWS_1M, &key).push(value);
    get_or_create_window(&TTFT_WINDOWS_10M, &key, 600);
    get_window_ref(&TTFT_WINDOWS_10M, &key).push(value);
    get_or_create_window(&TTFT_WINDOWS_1H, &key, 3600);
    get_window_ref(&TTFT_WINDOWS_1H, &key).push(value);
}

pub fn update_latency_windows(value: f64, model: &str, backend: &str) {
    let key = make_key(model, backend);
    get_or_create_window(&LATENCY_WINDOWS_1M, &key, 60);
    get_window_ref(&LATENCY_WINDOWS_1M, &key).push(value);
    get_or_create_window(&LATENCY_WINDOWS_10M, &key, 600);
    get_window_ref(&LATENCY_WINDOWS_10M, &key).push(value);
    get_or_create_window(&LATENCY_WINDOWS_1H, &key, 3600);
    get_window_ref(&LATENCY_WINDOWS_1H, &key).push(value);
}

pub fn update_tps_windows(value: f64, model: &str, backend: &str) {
    let key = make_key(model, backend);
    get_or_create_window(&TPS_WINDOWS_1M, &key, 60);
    get_window_ref(&TPS_WINDOWS_1M, &key).push(value);
    get_or_create_window(&TPS_WINDOWS_10M, &key, 600);
    get_window_ref(&TPS_WINDOWS_10M, &key).push(value);
    get_or_create_window(&TPS_WINDOWS_1H, &key, 3600);
    get_window_ref(&TPS_WINDOWS_1H, &key).push(value);
}

pub fn update_active_windows(value: f64, model: &str, backend: &str) {
    let key = make_key(model, backend);
    get_or_create_window(&ACTIVE_WINDOWS_1M, &key, 60);
    get_window_ref(&ACTIVE_WINDOWS_1M, &key).push(value);
    get_or_create_window(&ACTIVE_WINDOWS_10M, &key, 600);
    get_window_ref(&ACTIVE_WINDOWS_10M, &key).push(value);
    get_or_create_window(&ACTIVE_WINDOWS_1H, &key, 3600);
    get_window_ref(&ACTIVE_WINDOWS_1H, &key).push(value);
}

pub fn update_success_windows(success: bool, model: &str, backend: &str) {
    let key = make_key(model, backend);
    let value = if success { 1.0 } else { 0.0 };
    get_or_create_window(&SUCCESS_WINDOWS_1M, &key, 60);
    get_window_ref(&SUCCESS_WINDOWS_1M, &key).push(value);
    get_or_create_window(&SUCCESS_WINDOWS_10M, &key, 600);
    get_window_ref(&SUCCESS_WINDOWS_10M, &key).push(value);
    get_or_create_window(&SUCCESS_WINDOWS_1H, &key, 3600);
    get_window_ref(&SUCCESS_WINDOWS_1H, &key).push(value);
}

pub fn get_ttft_1m_max(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    TTFT_WINDOWS_1M.get(&key).map(|w| w.max()).unwrap_or(0.0)
}
pub fn get_ttft_10m_max(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    TTFT_WINDOWS_10M.get(&key).map(|w| w.max()).unwrap_or(0.0)
}
pub fn get_ttft_1h_max(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    TTFT_WINDOWS_1H.get(&key).map(|w| w.max()).unwrap_or(0.0)
}

pub fn get_latency_1m_max(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    LATENCY_WINDOWS_1M.get(&key).map(|w| w.max()).unwrap_or(0.0)
}
pub fn get_latency_10m_max(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    LATENCY_WINDOWS_10M
        .get(&key)
        .map(|w| w.max())
        .unwrap_or(0.0)
}
pub fn get_latency_1h_max(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    LATENCY_WINDOWS_1H.get(&key).map(|w| w.max()).unwrap_or(0.0)
}

pub fn get_tps_1m_avg(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    TPS_WINDOWS_1M.get(&key).map(|w| w.avg()).unwrap_or(0.0)
}
pub fn get_tps_10m_avg(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    TPS_WINDOWS_10M.get(&key).map(|w| w.avg()).unwrap_or(0.0)
}
pub fn get_tps_1h_avg(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    TPS_WINDOWS_1H.get(&key).map(|w| w.avg()).unwrap_or(0.0)
}

pub fn get_active_1m_max(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    ACTIVE_WINDOWS_1M.get(&key).map(|w| w.max()).unwrap_or(0.0)
}
pub fn get_active_10m_max(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    ACTIVE_WINDOWS_10M.get(&key).map(|w| w.max()).unwrap_or(0.0)
}
pub fn get_active_1h_max(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    ACTIVE_WINDOWS_1H.get(&key).map(|w| w.max()).unwrap_or(0.0)
}

pub fn get_success_1m(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    SUCCESS_WINDOWS_1M.get(&key).map(|w| w.avg()).unwrap_or(0.0)
}
pub fn get_success_10m(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    SUCCESS_WINDOWS_10M
        .get(&key)
        .map(|w| w.avg())
        .unwrap_or(0.0)
}
pub fn get_success_1h(model: &str, backend: &str) -> f64 {
    let key = make_key(model, backend);
    SUCCESS_WINDOWS_1H.get(&key).map(|w| w.avg()).unwrap_or(0.0)
}

pub fn update_success_overall(success: bool) {
    SUCCESS_WINDOW_OVERALL.push(if success { 1.0 } else { 0.0 });
}

pub fn get_success_overall() -> f64 {
    SUCCESS_WINDOW_OVERALL.avg()
}
