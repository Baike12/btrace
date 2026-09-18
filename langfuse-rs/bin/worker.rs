//! Langfuse Worker — 后台队列消费者 (queue consumers only)
//!
//! Runs exactly the same consumers as `bin/server.rs` (which hosts them
//! in-process) without the HTTP API. Useful when the queue load should be scaled
//! or restarted independently of the API.
//!
//! **Do not run this alongside `bin/server.rs` against the same database unless
//! you intend to double the consumer count**: both register the same queues, so
//! each queue ends up with two pools of workers competing for the same `pg_jobs`
//! rows. The container image ships only `server`.
//!
//! 用法:
//!   cargo run --bin worker
//!
//! 环境变量:
//!   DATABASE_URL - PostgreSQL 连接串 (required)
//!   RUST_LOG - 日志级别 (默认: info,langfuse=debug)

use langfuse::queues::register_all_queues;
use langfuse_core::Config;
use langfuse_db::pool::init_pool;
use langfuse_queue::WorkerManager;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化 tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,langfuse=debug".into()),
        )
        .init();

    // 加载 .env 文件
    dotenvy::dotenv().ok();

    let config = Config::from_env();

    tracing::info!("Connecting to database...");
    let pool = init_pool(&config.database_url).await?;
    tracing::info!("Database connected");

    // 健康检查
    langfuse_db::pool::check_health(&pool).await?;
    tracing::info!("Database health check passed");

    let mut manager = WorkerManager::new(pool.clone());
    register_all_queues(&mut manager, &pool, &config);

    tracing::info!("All workers registered, starting event loop...");
    tracing::info!("Press Ctrl-C to stop");

    // 运行所有 worker (阻塞直到 ctrl-c)
    manager.run_all().await;

    tracing::info!("Worker shut down gracefully");
    Ok(())
}
