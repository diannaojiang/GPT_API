use std::fs;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{filter, fmt, EnvFilter, Layer};

/// 日志系统配置
pub struct LogConfig {
    pub log_dir: String,
    pub general_log_retention_days: usize,
    pub error_log_retention_days: usize,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            log_dir: "logs".to_string(),
            general_log_retention_days: 10,
            error_log_retention_days: 30,
        }
    }
}

/// 初始化日志系统
///
/// 返回一个WorkerGuard向量，必须在程序运行期间保持活跃以确保日志正确刷新
pub fn init_logging(config: LogConfig) -> Vec<WorkerGuard> {
    // 确保日志目录存在
    fs::create_dir_all(&config.log_dir).expect("Failed to create log directory");

    let mut guards = Vec::new();

    // 创建通用日志的非阻塞写入器（INFO, WARNING等）
    // 使用每日轮转
    let general_file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("info") // 改为 info
        .filename_suffix("log")
        .max_log_files(config.general_log_retention_days)
        .build(&config.log_dir)
        .expect("Failed to create general log appender");

    let (general_non_blocking, general_guard) =
        tracing_appender::non_blocking(general_file_appender);
    guards.push(general_guard);

    // 创建错误日志的非阻塞写入器（每日轮转）
    let error_file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY) // 每日轮转
        .filename_prefix("error")
        .filename_suffix("log")
        .max_log_files(config.error_log_retention_days)
        .build(&config.log_dir)
        .expect("Failed to create error log appender");

    let (error_non_blocking, error_guard) = tracing_appender::non_blocking(error_file_appender);
    guards.push(error_guard);

    // 创建控制台输出的非阻塞写入器
    let (console_non_blocking, console_guard) = tracing_appender::non_blocking(std::io::stdout());
    guards.push(console_guard);

    // 为不同日志级别创建层
    let general_layer = fmt::layer()
        .with_writer(general_non_blocking)
        .with_ansi(false) // 在文件日志中禁用ANSI颜色
        .with_filter(filter::filter_fn(|meta| {
            // 只包含INFO, WARN, DEBUG级别（排除ERROR）
            !matches!(meta.level().as_str(), "ERROR")
        }));

    let error_layer = fmt::layer()
        .with_writer(error_non_blocking)
        .with_ansi(false) // 在文件日志中禁用ANSI颜色
        .with_filter(filter::filter_fn(|meta| {
            // 只包含ERROR级别
            meta.level().as_str() == "ERROR"
        }));

    let console_layer = fmt::layer()
        .with_writer(console_non_blocking)
        .with_filter(EnvFilter::from_default_env());

    // 构建订阅者
    tracing_subscriber::registry()
        .with(general_layer)
        .with(error_layer)
        .with(console_layer)
        .init();

    guards
}
