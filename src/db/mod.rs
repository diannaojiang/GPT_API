use chrono::{Datelike, Local};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use std::fs;
use std::path::Path;
use std::str::FromStr;
use tracing::{debug, error, info, warn};

use crate::config::types::Config;
use crate::state::app_state::AppState;
use std::sync::Arc;

pub mod records;

/// 初始化数据库连接池
pub async fn init_db_pool(_config: &Config) -> Result<SqlitePool, sqlx::Error> {
    let database_url =
        std::env::var("RECD_PATH").unwrap_or_else(|_| "sqlite:./record.db".to_string());

    let db_path = database_url
        .strip_prefix("sqlite:")
        .unwrap_or(&database_url);

    let options = SqliteConnectOptions::from_str(db_path)?.create_if_missing(true);

    let pool = SqlitePool::connect_with(options).await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS records (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            Time TEXT,
            IP TEXT,
            Model TEXT,
            Type TEXT,
            CompletionTokens INTEGER,
            PromptTokens INTEGER,
            TotalTokens INTEGER,
            Tool BOOLEAN,
            Multimodal BOOLEAN,
            Headers TEXT,
            Request TEXT,
            Response TEXT
        )
        "#,
    )
    .execute(&pool)
    .await?;

    info!("Database at '{}' initialized successfully", db_path);
    Ok(pool)
}

/// 检查并轮换数据库
/// 归档上个月的数据：检查是否存在上个月的归档文件，不存在则轮转
pub async fn check_and_rotate(app_state: &Arc<AppState>) {
    let _lock = app_state.db_rotation_lock.lock().await;
    let db_path_with_prefix =
        std::env::var("RECD_PATH").unwrap_or_else(|_| "sqlite:./record.db".to_string());

    // Strip the "sqlite:" prefix to get the actual file path
    let db_path_str = db_path_with_prefix
        .strip_prefix("sqlite:")
        .unwrap_or(&db_path_with_prefix);

    warn!("[DB ROTATION] Checking database at: {}", db_path_str);

    let db_path = Path::new(db_path_str);

    if !db_path.exists() {
        debug!("[DB ROTATION] Database file does not exist, skipping");
        return;
    }

    let db_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let now = Local::now();

    // 计算上个月的年月
    let (last_month_year, last_month) = if now.month() == 1 {
        (now.year() - 1, 12)
    } else {
        (now.year(), now.month() - 1)
    };
    let last_month_str = format!("{}{:02}", last_month_year, last_month);

    warn!(
        "[DB ROTATION] Current: {}/{}, Looking for: record_{}",
        now.year(),
        now.month(),
        last_month_str
    );

    // 检查是否存在上个月的归档文件
    let needs_rotation = match fs::read_dir(db_dir) {
        Ok(entries) => {
            let entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            let has_last_month_archive = entries
                .iter()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .any(|name| name.starts_with(&format!("record_{}", last_month_str)));

            warn!(
                "[DB ROTATION] Found {} files in dir, has_last_month_archive: {}",
                entries.len(),
                has_last_month_archive
            );

            !has_last_month_archive
        }
        Err(e) => {
            error!("[DB ROTATION] Failed to read directory: {}", e);
            true
        }
    };

    if !needs_rotation {
        debug!("[DB ROTATION] No rotation needed, archive exists");
        return;
    }

    let archive_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let archive_filename = format!("record_{}.db", last_month_str);
    let mut archive_path = archive_dir.join(&archive_filename);

    info!(
        "Database rotation needed (archiving {} data). Archiving {} to {}",
        last_month_str,
        db_path.display(),
        archive_path.display()
    );

    if archive_path.exists() {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let new_archive_filename = format!("record_{}_{}.db", last_month_str, timestamp);
        archive_path = archive_dir.join(new_archive_filename);
        warn!(
            "Archive file already exists. Renaming to {}",
            archive_path.display()
        );
    }

    // Acquire write lock to block new requests and safely rotate
    let mut pool_guard = app_state.db_pool.write().await;

    // Close the current connection pool to release file locks
    pool_guard.close().await;

    match fs::rename(db_path, &archive_path) {
        Ok(_) => info!("Database archived successfully."),
        Err(e) => {
            error!("Failed to archive database: {}", e);
        }
    }

    info!("Re-initializing database pool after rotation.");
    let config = app_state.config_manager.get_config().await;
    match init_db_pool(&config).await {
        Ok(new_pool) => {
            *pool_guard = new_pool;
            info!("Database pool re-initialized successfully.");
        }
        Err(e) => {
            error!(
                "Failed to re-initialize database pool after rotation: {}",
                e
            );
        }
    }
}
