use super::types::{Config, ExtraBodyCached};
use notify::{
    recommended_watcher, Event, RecommendedWatcher, RecursiveMode, Result as NotifyResult, Watcher,
};
use serde_yaml;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard};
use tracing::{error, info};

pub struct ConfigManager {
    config: Arc<RwLock<Config>>,
    _config_path: String,         // 保留路径以备后用，添加 _ 前缀消除未使用警告
    _watcher: RecommendedWatcher, // Keep watcher alive
}

impl ConfigManager {
    /// 创建新的 ConfigManager 并开始监听配置文件变化
    pub async fn new(config_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = Self::load_config(config_path)?;
        config.config_generation = 1;
        let config_arc = Arc::new(RwLock::new(config));

        // Initialize watching immediately to get the watcher instance
        let watcher = Self::setup_watcher(config_path, config_arc.clone()).await?;

        let manager = ConfigManager {
            config: config_arc,
            _config_path: config_path.to_string(),
            _watcher: watcher,
        };

        Ok(manager)
    }

    pub fn load_config(config_path: &str) -> Result<Config, Box<dyn std::error::Error>> {
        let contents = fs::read_to_string(config_path)?;
        let mut config: Config = serde_yaml::from_str(&contents)?;
        Self::populate_extra_body_cache(&mut config);
        Ok(config)
    }

    /// Parse each client's `extra_body` JSON string into the cached Value tree
    /// so that runtime hot path avoids per-request serde_json::from_str.
    fn populate_extra_body_cache(config: &mut Config) {
        for client in &mut config.openai_clients {
            let parsed = client.extra_body.as_deref().and_then(|raw| {
                if raw.trim().is_empty() {
                    return None;
                }
                match serde_json::from_str(raw) {
                    Ok(serde_json::Value::Object(map)) => Some(map),
                    _ => {
                        tracing::warn!(
                            "Client '{}' extra_body is not a valid JSON object, skipping",
                            client.name
                        );
                        None
                    }
                }
            });
            client.extra_body_cached = ExtraBodyCached(parsed);
        }
    }

    pub async fn get_config(&self) -> Config {
        self.config.read().await.clone()
    }

    pub async fn get_config_guard(&self) -> RwLockReadGuard<'_, Config> {
        self.config.read().await
    }

    /// Return a consistent snapshot of (Config, config_generation) for use by the cache.
    /// The generation is read atomically with the config contents so they always match.
    pub async fn get_config_with_generation(&self) -> (Config, u64) {
        let guard = self.config.read().await;
        let gen = guard.config_generation;
        (guard.clone(), gen)
    }

    async fn setup_watcher(
        config_path_str: &str,
        config: Arc<RwLock<Config>>,
    ) -> NotifyResult<RecommendedWatcher> {
        let config_path = config_path_str.to_string();
        let config_path_for_check = config_path.clone();

        // Capture the runtime handle to submit tasks from the non-async watcher thread
        let runtime_handle = tokio::runtime::Handle::current();

        // Create a watcher object
        let mut watcher = recommended_watcher(move |res: NotifyResult<Event>| {
            match res {
                Ok(event) => {
                    // Check if the event is for our config file
                    // Using loose matching because editors often save to temp files and rename
                    if event
                        .paths
                        .iter()
                        .any(|p| p.to_string_lossy().contains(&config_path_for_check))
                    {
                        info!("Config file changed, reloading...");
                        // Add a small delay to allow file write to complete
                        std::thread::sleep(std::time::Duration::from_millis(100));

                        match Self::load_config(&config_path_for_check) {
                            Ok(new_config) => {
                                let config_clone = config.clone();
                                // Use the captured handle to spawn the async task
                                runtime_handle.spawn(async move {
                                    let mut guard = config_clone.write().await;
                                    // Increment generation atomically with the config swap
                                    let next_gen = guard.config_generation.wrapping_add(1);
                                    let mut fresh = new_config;
                                    fresh.config_generation = next_gen;
                                    *guard = fresh;
                                    info!(
                                        "Config reloaded successfully (generation {}).",
                                        next_gen
                                    );
                                });
                            }
                            Err(e) => {
                                error!("Failed to reload config: {}", e);
                            }
                        }
                    }
                }
                Err(e) => error!("watch error: {:?}", e),
            }
        })?;

        // Add a path to be watched.
        // Watch the parent directory to handle atomic saves (rename/move) better
        let path_to_watch = Path::new(config_path_str)
            .parent()
            .unwrap_or(Path::new("."));

        watcher.watch(path_to_watch, RecursiveMode::NonRecursive)?;

        info!(
            "Started watching config file directory: {:?}",
            path_to_watch
        );
        Ok(watcher)
    }
}
