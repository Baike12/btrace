//! Langfuse Unified Backend — REST API + Queue Consumers (single process)

use langfuse::queues::register_all_queues;
use langfuse_api::app::{self, AppState};
use langfuse_core::Config;
use langfuse_db::pool::init_pool;
use langfuse_queue::WorkerManager;
use std::sync::Arc;
use tokio::sync::watch;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,langfuse=debug".into()),
        )
        .init();

    dotenvy::dotenv().ok();

    let config = Config::from_env();

    tracing::info!("Connecting to database...");
    let pool = init_pool(&config.database_url).await?;
    langfuse_db::pool::check_health(&pool).await?;
    tracing::info!("Database connected");

    // Run startup bootstrap (auto-provision org/project/user/api-key)
    langfuse_db::bootstrap::run_bootstrap(&pool).await?;

    // Shutdown coordination: broadcast channel
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Spawn all queue consumers in a background tokio task
    let worker_pool = pool.clone();
    let worker_config = config.clone();
    let mut worker_shutdown = shutdown_rx.clone();
    let worker_handle = tokio::spawn(async move {
        let mut manager = WorkerManager::new(worker_pool.clone());
        register_all_queues(&mut manager, &worker_pool, &worker_config);

        // Run workers until shutdown signal
        tokio::select! {
            _ = manager.run_all() => {}
            _ = worker_shutdown.changed() => {
                tracing::info!("Workers: shutdown signal received, draining...");
            }
        }
        tracing::info!("Workers: shut down");
    });

    // API server（主线程）
    let state = AppState {
        pool: pool.clone(),
        jwt_secret: Arc::new(config.jwt_secret.clone().into_bytes()),
        worker: None,
        login_throttle: Arc::new(langfuse_api::throttle::LoginThrottle::new()),
    };

    tracing::info!(
        "Langfuse backend starting (API + Workers) on {}:{}",
        config.hostname,
        config.port
    );

    // Signal to catch: Ctrl-C
    let shutdown_signal = async {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Received Ctrl-C, starting graceful shutdown...");
    };

    app::serve(state, config.port, &config.hostname, shutdown_signal).await?;

    // Signal workers to stop and wait briefly for them to finish
    let _ = shutdown_tx.send(true);
    match tokio::time::timeout(std::time::Duration::from_secs(10), worker_handle).await {
        Ok(Ok(())) => tracing::info!("Workers joined cleanly"),
        Ok(Err(e)) => tracing::error!("Worker task panicked: {:?}", e),
        Err(_) => tracing::warn!("Workers did not finish within 10s, exiting anyway"),
    }

    tracing::info!("Langfuse backend shut down gracefully");
    Ok(())
}
