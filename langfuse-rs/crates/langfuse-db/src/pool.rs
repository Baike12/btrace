use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

/// 初始化 PostgreSQL 连接池
///
/// 连接池配置：
/// - max_connections=20（原 Prisma 默认 num_cpus×2+1≈46，现在明确限制）
/// - min_connections=2（保持最小连接以减少冷启动延迟）
/// - idle_timeout=5min（空闲连接超时）
/// - max_lifetime=30min（连接最大生命周期）
pub async fn init_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(20)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(300))
        .max_lifetime(Duration::from_secs(1800))
        .connect(database_url)
        .await
}

/// 健康检查：ping 数据库
pub async fn check_health(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}
