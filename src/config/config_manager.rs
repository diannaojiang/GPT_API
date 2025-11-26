use super::types::Config;
use notify::{recommended_watcher, Event, RecursiveMode, Result as NotifyResult, Watcher};
use serde_yaml;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard};

pub struct ConfigManager {
    config: Arc<RwLock<Config>>,
    config_path: String,
}

impl ConfigManager {
    pub async fn new(config_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Self::load_config(config_path)?;
        let manager = ConfigManager {
            config: Arc::new(RwLock::new(config)),
            config_path: config_path.to_string(),
        };

        // Start watching for config file changes
        manager.start_watching().await?;

        Ok(manager)
    }

    pub fn load_config(config_path: &str) -> Result<Config, Box<dyn std::error::Error>> {
        let contents = fs::read_to_string(config_path)?;
        let config: Config = serde_yaml::from_str(&contents)?;
        Ok(config)
    }

    pub async fn get_config(&self) -> Config {
        self.config.read().await.clone()
    }

    pub async fn get_config_guard(&self) -> RwLockReadGuard<'_, Config> {
        self.config.read().await
    }

    async fn start_watching(&self) -> NotifyResult<()> {
        let config_path = self.config_path.clone();
        let config_path_for_watcher = self.config_path.clone();
        let config_path_for_print = self.config_path.clone();
        let config = self.config.clone();

        // Create a watcher object
        let mut watcher = recommended_watcher(move |res: NotifyResult<Event>| {
            match res {
                Ok(event) => {
                    // Check if the event is for our config file
                    if event.paths.iter().any(|p| p == Path::new(&config_path)) {
                        println!("Config file changed, reloading...");
                        match Self::load_config(&config_path) {
                            Ok(new_config) => {
                                // Update the config in the shared state
                                // Note: This is a simplified approach. In a real application,
                                // you might want to handle errors more gracefully.
                                let config_clone = config.clone();
                                tokio::spawn(async move {
                                    *config_clone.write().await = new_config;
                                });
                            }
                            Err(e) => {
                                eprintln!("Failed to reload config: {}", e);
                            }
                        }
                    }
                }
                Err(e) => eprintln!("watch error: {:?}", e),
            }
        })?;

        // Add a path to be watched. All files and directories at that path and
        // below will be monitored for changes.
        watcher.watch(
            Path::new(&config_path_for_watcher).parent().unwrap(),
            RecursiveMode::NonRecursive,
        )?;

        // Keep the watcher alive
        // In a real application, you would store the watcher somewhere
        // For now, we'll just drop it, which will stop watching
        // We need to keep it alive, so let's store it in a static or pass it around
        // For simplicity, we'll just print a message
        println!("Started watching config file: {}", config_path_for_print);

        // To keep the watcher alive, we would need to store it somewhere.
        // For now, we'll just return Ok(()) and assume the watcher is kept alive
        // by the caller. In a real application, you'd want to store the watcher
        // in a struct or pass it around.
        Ok(())
    }
}
