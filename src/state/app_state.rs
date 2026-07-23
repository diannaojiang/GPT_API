use crate::client::client_manager::ClientManager;
use crate::config::config_manager::ConfigManager;
use crate::services::dispatcher::DispatcherService;
use crate::services::models_cache::ModelsCache;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

pub struct AppState {
    pub config_manager: Arc<ConfigManager>,
    pub client_manager: Arc<ClientManager>,
    pub dispatcher_service: DispatcherService,
    pub db_pool: RwLock<SqlitePool>,
    pub db_rotation_lock: Mutex<()>,
    pub models_cache: ModelsCache,
}

impl AppState {
    pub fn new(
        config_manager: Arc<ConfigManager>,
        client_manager: Arc<ClientManager>,
        db_pool: SqlitePool,
    ) -> Self {
        let dispatcher_service =
            DispatcherService::new(config_manager.clone(), client_manager.clone());
        AppState {
            config_manager,
            client_manager,
            dispatcher_service,
            db_pool: RwLock::new(db_pool),
            db_rotation_lock: Mutex::new(()),
            models_cache: ModelsCache::new(),
        }
    }
}
