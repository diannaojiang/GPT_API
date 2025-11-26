use crate::client::client_manager::ClientManager;
use crate::config::config_manager::ConfigManager;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

pub struct AppState {
    pub config_manager: Arc<ConfigManager>,
    pub client_manager: Arc<ClientManager>,
    pub db_pool: RwLock<SqlitePool>,
    pub db_rotation_lock: Mutex<()>,
}

impl AppState {
    pub fn new(
        config_manager: Arc<ConfigManager>,
        client_manager: Arc<ClientManager>,
        db_pool: SqlitePool,
    ) -> Self {
        AppState {
            config_manager,
            client_manager,
            db_pool: RwLock::new(db_pool),
            db_rotation_lock: Mutex::new(()),
        }
    }
}
