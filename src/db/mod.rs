use chrono::Local;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use std::fs;
use std::path::Path;
use std::str::FromStr;
use tracing::{error, info, warn};

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
/// 通过检查是否存在当月的归档文件来判断是否需要轮转
pub async fn check_and_rotate(app_state: &Arc<AppState>) {
    let _lock = app_state.db_rotation_lock.lock().await;
    let db_path_str = std::env::var("RECD_PATH").unwrap_or_else(|_| "./record.db".to_string());
    let db_path = Path::new(&db_path_str);

    if !db_path.exists() {
        return;
    }

    let db_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let now = Local::now();
    let current_month_str = now.format("%Y%m").to_string();

    // Check if there are archives from current month (meaning we already rotated this month)
    let needs_rotation = match fs::read_dir(db_dir) {
        Ok(entries) => {
            let has_current_month_archive = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .any(|name| name.starts_with(&format!("record_{}", current_month_str)));
            !has_current_month_archive
        }
        Err(_) => true,
    };

    if !needs_rotation {
        return;
    }

    let archive_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let archive_filename = format!("record_{}.db", current_month_str);
    let mut archive_path = archive_dir.join(&archive_filename);

    info!(
        "Database rotation needed. Archiving {} to {}",
        db_path.display(),
        archive_path.display()
    );

    if archive_path.exists() {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let new_archive_filename = format!("record_{}_{}.db", current_month_str, timestamp);
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
