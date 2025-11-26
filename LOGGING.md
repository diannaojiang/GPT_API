# Rust GPT_API Logging System

This document describes the logging system implemented for the Rust GPT_API, which matches the capabilities of the Python version.

## Features

1. **File-based logging with rotation**:
   - General logs (INFO, WARNING) to `logs/openai-api.log` with daily rotation
   - Error logs to `logs/error_{time:YYYY-MM-DD}.log` with daily rotation

2. **Log retention policies**:
   - General logs: retain for 10 days
   - Error logs: retain for 30 days

3. **Asynchronous logging capabilities**:
   - Uses non-blocking writers for better performance

4. **Easy configuration**:
   - Integrates with the existing tracing setup
   - Console output with environment-based filtering

## Implementation Details

### Dependencies

The implementation uses the following crates:
- `tracing` - Core tracing framework
- `tracing-subscriber` - Subscriber implementation with filtering capabilities
- `tracing-appender` - File appenders with rotation support

### Configuration

The logging system is configured through the `LogConfig` struct:

```rust
pub struct LogConfig {
    pub log_dir: String,                // Directory for log files
    pub general_log_retention_days: usize,  // Retention for general logs
    pub error_log_retention_days: usize,    // Retention for error logs
}
```

### Log Separation

Logs are separated by level using custom filters:
- General log layer: Accepts INFO, WARN, and DEBUG levels (excludes ERROR)
- Error log layer: Accepts only ERROR level
- Console layer: Uses environment-based filtering

### Limitations

The current implementation uses daily rotation for both general and error logs since `tracing-appender` doesn't support size-based rotation. This is a compromise from the Python version which uses 100MB size-based rotation for general logs.

To implement size-based rotation, we would need to use a different crate like `log4rs` or implement a custom solution.

## Usage

The logging system is initialized in `main.rs`:

```rust
// Initialize our custom logging system
let log_config = LogConfig::default();
let _guards = init_logging(log_config); // Keep guards alive to ensure logs are flushed
```

The `_guards` must be kept alive for the duration of the program to ensure logs are properly flushed to files.