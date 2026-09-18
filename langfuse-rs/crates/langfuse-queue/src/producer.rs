use langfuse_core::{BackoffType, Result};
use serde_json::Value as JsonValue;
use sqlx::PgPool;

/// 队列选项
#[derive(Debug, Clone)]
pub struct QueueOptions {
    pub delay_ms: i64,
    pub max_attempts: i32,
    pub backoff_type: Option<BackoffType>,
    pub backoff_delay_ms: Option<i32>,
}

impl Default for QueueOptions {
    fn default() -> Self {
        Self {
            delay_ms: 0,
            max_attempts: 5,
            backoff_type: Some(BackoffType::Exponential),
            backoff_delay_ms: Some(5000),
        }
    }
}

/// PgQueue — PG 原生队列生产者
pub struct PgQueue {
    pool: PgPool,
    queue_name: String,
}

impl PgQueue {
    pub fn new(pool: PgPool, queue_name: impl Into<String>) -> Self {
        Self {
            pool,
            queue_name: queue_name.into(),
        }
    }

    /// 入队一个 job
    pub async fn add(
        &self,
        _name: &str,
        payload: &JsonValue,
        opts: &QueueOptions,
    ) -> Result<()> {
        let job_id = uuid::Uuid::new_v4();
        let backoff_type_str = opts.backoff_type.as_ref().map(|b| b.to_string());

        sqlx::query(
            r#"INSERT INTO pg_jobs (
                job_id, queue_name, payload, state, run_at,
                max_attempts, backoff_type, backoff_delay
            ) VALUES ($1, $2, $3, 'waiting', now() + ($4::bigint || ' milliseconds')::interval, $5, $6, $7)"#,
        )
        .bind(job_id)
        .bind(&self.queue_name)
        .bind(payload)
        .bind(opts.delay_ms)
        .bind(opts.max_attempts)
        .bind(backoff_type_str)
        .bind(opts.backoff_delay_ms)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// 统计等待中的 job 数量
    pub async fn waiting_count(&self) -> Result<i64> {
        let row = sqlx::query_scalar::<_, Option<i64>>(
            r#"SELECT COUNT(*) FROM pg_jobs
            WHERE queue_name = $1 AND state = 'waiting'"#,
        )
        .bind(&self.queue_name)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.unwrap_or(0))
    }

    /// 暂停队列
    pub async fn pause(&self) -> Result<u64> {
        let result = sqlx::query(
            r#"UPDATE pg_jobs SET state = 'delayed'
            WHERE queue_name = $1 AND state = 'waiting'"#,
        )
        .bind(&self.queue_name)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// 恢复队列
    pub async fn resume(&self) -> Result<u64> {
        let result = sqlx::query(
            r#"UPDATE pg_jobs SET state = 'waiting'
            WHERE queue_name = $1 AND state = 'delayed'"#,
        )
        .bind(&self.queue_name)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// 清理已完成/失败的旧 job
    pub async fn clean(&self, grace_period_ms: i64) -> Result<u64> {
        let cutoff = chrono::Utc::now() - chrono::Duration::milliseconds(grace_period_ms);
        let result = sqlx::query(
            r#"DELETE FROM pg_jobs
            WHERE queue_name = $1
              AND state IN ('completed', 'failed')
              AND finished_at < $2"#,
        )
        .bind(&self.queue_name)
        .bind(cutoff)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}
