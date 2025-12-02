use axum::{extract::State, middleware as axum_middleware};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing_subscriber::{self, EnvFilter};

mod client;
mod config;
mod db;
mod handlers;
// mod middleware; // Removed, use lib's middleware
mod models;
mod routes;
mod state;

use gpt_api::middleware; // Import from library crate

use crate::db::{check_and_rotate, init_db_pool};
use client::client_manager::ClientManager;
use state::app_state::AppState;

async fn rotation_middleware(
    State(state): State<Arc<AppState>>,
    request: axum::http::Request<axum::body::Body>,
    next: axum_middleware::Next,
) -> axum::response::Response {
    check_and_rotate(&state).await;
    next.run(request).await
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(128)
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        // Initialize tracing
        // 配置自定义的日志系统 (File + Console)
        let log_config = gpt_api::logging::LogConfig::default();
        let _guards = gpt_api::logging::init_logging(log_config);

        // Load configuration
        let config_manager = config::config_manager::ConfigManager::new("config/config.yaml")
            .await
            .expect("Failed to load configuration");

        let config_manager = Arc::new(config_manager);

        // Initialize client manager
        let client_manager = Arc::new(ClientManager::new());
        let config = config_manager.get_config().await;

        // Initialize database pool
        let db_pool = init_db_pool(&config)
            .await
            .expect("Failed to initialize database pool");

        let app_state = Arc::new(AppState::new(config_manager, client_manager, db_pool));

        // Add a small delay to ensure the database is fully initialized on disk
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let app = routes::create_router(app_state.clone())
            .layer(axum_middleware::from_fn_with_state(
                app_state.clone(),
                rotation_middleware,
            ))
            .layer(axum_middleware::from_fn(
                middleware::access_log::access_log_middleware,
            )); // Access Log 最外层

        // Run our app with hyper, listening globally on port 8000 or SERVER_PORT env var
        let port = std::env::var("SERVER_PORT")
            .unwrap_or_else(|_| "8000".to_string())
            .parse()
            .expect("SERVER_PORT must be a number");
        // Listen on IPv6 "any" address (::), which generally also supports IPv4 (dual-stack)
        let addr = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 0], port));
        let listener = TcpListener::bind(addr).await.unwrap();
        tracing::info!("listening on {}", addr);
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
}
