use chrono::{DateTime, Datelike, Local};
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
pub async fn check_and_rotate(app_state: &Arc<AppState>) {
    // debug!("Checking for database rotation...");
    let _lock = app_state.db_rotation_lock.lock().await;
    let db_path_str = std::env::var("RECD_PATH").unwrap_or_else(|_| "./record.db".to_string());
    let db_path = Path::new(&db_path_str);

    if !db_path.exists() {
        // debug!("Database file does not exist: {}", db_path.display());
        return;
    }

    let metadata = match fs::metadata(db_path) {
        Ok(meta) => meta,
        Err(e) => {
            error!("Failed to get metadata for database file: {}", e);
            return;
        }
    };

    let mod_time = match metadata.modified() {
        Ok(time) => time,
        Err(e) => {
            error!("Failed to get modification time for database file: {}", e);
            return;
        }
    };

    let mod_datetime: DateTime<Local> = mod_time.into();
    let now = Local::now();

    if mod_datetime.year() != now.year() || mod_datetime.month() != now.month() {
        let archive_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
        let archive_filename = format!("record_{}.db", mod_datetime.format("%Y%m"));
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
            let new_archive_filename =
                format!("record_{}_{}.db", mod_datetime.format("%Y%m"), timestamp);
            archive_path = archive_dir.join(new_archive_filename);
            warn!(
                "Archive file already exists. Renaming to {}",
                archive_path.display()
            );
        }

        // Acquire write lock to block new requests and safely rotate
        let mut pool_guard = app_state.db_pool.write().await;

        // Close the current connection pool to release file locks (important for Windows, good practice elsewhere)
        pool_guard.close().await;

        match fs::rename(db_path, &archive_path) {
            Ok(_) => info!("Database archived successfully."),
            Err(e) => {
                error!("Failed to archive database: {}", e);
                // Attempt to restore pool? Or just let it fail and next retry will fix?
                // Ideally we should re-open the original db if rename fails.
                // For now, we try to re-init anyway to recover service.
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
}
