use async_trait::async_trait;
use langfuse_core::PgJob;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

/// Job 处理器 trait — 每个队列实现自己的 process 逻辑
#[async_trait]
pub trait JobProcessor: Send + Sync {
    async fn process(&self, job: &PgJob) -> Result<(), String>;
}

/// PgWorker — PG 原生队列消费者
pub struct PgWorker {
    queue_name: String,
    pool: PgPool,
    processor: Arc<dyn JobProcessor>,
    concurrency: usize,
    poll_interval: Duration,
    stalled_interval: Duration,
    max_attempts: i32,
    worker_id: String,
}

impl PgWorker {
    pub fn new(
        queue_name: impl Into<String>,
        pool: PgPool,
        processor: Arc<dyn JobProcessor>,
        concurrency: usize,
    ) -> Self {
        Self {
            queue_name: queue_name.into(),
            pool,
            processor,
            concurrency,
            poll_interval: Duration::from_millis(500),
            stalled_interval: Duration::from_secs(120),
            max_attempts: 6,
            worker_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub fn with_poll_interval(mut self, ms: u64) -> Self {
        self.poll_interval = Duration::from_millis(ms);
        self
    }

    pub fn with_max_attempts(mut self, max: i32) -> Self {
        self.max_attempts = max;
        self
    }

    /// 启动 worker 主循环
    pub async fn run(self: Arc<Self>) {
        let worker = self;
        let semaphore = Arc::new(Semaphore::new(worker.concurrency));

        tracing::info!(
            "PgWorker started: {} (concurrency={})",
            worker.queue_name,
            worker.concurrency
        );

        // 主工作循环：如果 poll 或 recovery loop 意外退出则自动重启
        loop {
            // 主轮询循环
            let poll_worker = worker.clone();
            let poll_sem = semaphore.clone();
            let poll_handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(poll_worker.poll_interval);
                loop {
                    interval.tick().await;
                    match poll_worker.poll(&poll_sem).await {
                        Ok(jobs) => {
                            for job in jobs {
                                let permit = poll_sem.clone().acquire_owned().await;
                                match permit {
                                    Ok(permit) => {
                                        let w = poll_worker.clone();
                                        tokio::spawn(async move {
                                            let _guard = permit;
                                            w.execute_job(job).await;
                                        });
                                    }
                                    Err(_) => {
                                        tracing::error!("Semaphore closed, worker shutting down");
                                        return;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("Poll error in {}: {}", poll_worker.queue_name, e);
                        }
                    }
                }
            });

            // 停滞恢复循环
            let recovery_worker = worker.clone();
            let recovery_handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(recovery_worker.stalled_interval);
                loop {
                    interval.tick().await;
                    if let Err(e) = recovery_worker.recover_stalled_jobs().await {
                        tracing::error!("Stalled recovery error: {}", e);
                    }
                }
            });

            // 等待任意循环退出，然后重启
            tokio::select! {
                result = poll_handle => {
                    match result {
                        Ok(()) => tracing::warn!("Poll loop for {} exited normally, restarting...", worker.queue_name),
                        Err(e) => tracing::error!("Poll loop for {} panicked: {:?}, restarting...", worker.queue_name, e),
                    }
                }
                result = recovery_handle => {
                    match result {
                        Ok(()) => tracing::warn!("Recovery loop for {} exited normally, restarting...", worker.queue_name),
                        Err(e) => tracing::error!("Recovery loop for {} panicked: {:?}, restarting...", worker.queue_name, e),
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("PgWorker {} received SIGTERM, draining...", worker.queue_name);
                    worker.drain(&semaphore).await;
                    break; // 退出外层 loop，优雅关闭
                }
            }

            // 短暂延迟后重启，避免 CPU 紧循环
            tokio::time::sleep(Duration::from_secs(1)).await;
            tracing::info!("PgWorker {} restarting loops...", worker.queue_name);
        }
    }

    /// 核心轮询：SELECT ... FOR UPDATE SKIP LOCKED
    async fn poll(&self, semaphore: &Semaphore) -> Result<Vec<PgJob>, sqlx::Error> {
        let available = semaphore.available_permits();
        if available == 0 {
            return Ok(vec![]);
        }

        let take = (self.concurrency - available).clamp(1, 10);

        sqlx::query_as::<_, PgJob>(
            r#"WITH next_job AS (
                SELECT id FROM pg_jobs
                WHERE queue_name = $1
                  AND state = 'waiting'
                  AND run_at <= now()
                ORDER BY run_at ASC
                LIMIT $2
                FOR UPDATE SKIP LOCKED
            )
            UPDATE pg_jobs
            SET state = 'active',
                locked_by = $3,
                locked_until = now() + interval '5 minutes',
                started_at = now(),
                attempts = attempts + 1
            FROM next_job
            WHERE pg_jobs.id = next_job.id
            RETURNING pg_jobs.*"#,
        )
        .bind(&self.queue_name)
        .bind(take as i64)
        .bind(&self.worker_id)
        .fetch_all(&self.pool)
        .await
    }

    /// 执行单个 job
    async fn execute_job(self: &Arc<Self>, job: PgJob) {
        let result = self.processor.process(&job).await;
        match result {
            Ok(()) => {
                if let Err(e) = self.mark_completed(job.id).await {
                    tracing::error!("Failed to mark job {} completed: {}", job.id, e);
                }
            }
            Err(err_msg) => {
                tracing::warn!("Job {} failed (attempt {}/{}): {}",
                    job.id, job.attempts + 1, job.max_attempts, err_msg);
                self.handle_failure(&job, &err_msg).await;
            }
        }
    }

    /// 标记 job 完成
    async fn mark_completed(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE pg_jobs SET state = 'completed', finished_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 处理失败：重试或永久失败
    async fn handle_failure(&self, job: &PgJob, error: &str) {
        let total_attempts = job.attempts + 1;

        if total_attempts >= job.max_attempts {
            let result = sqlx::query(
                "UPDATE pg_jobs SET state = 'failed', finished_at = now(), last_error = $2 WHERE id = $1",
            )
            .bind(job.id)
            .bind(error)
            .execute(&self.pool)
            .await;

            match result {
                Ok(_) => tracing::error!("Job {} permanently failed after {} attempts", job.id, total_attempts),
                Err(e) => tracing::error!("Failed to mark job {} as failed: {}", job.id, e),
            }
        } else {
            let delay_ms = match job.backoff_type.as_deref() {
                Some("exponential") => {
                    let base = job.backoff_delay.unwrap_or(5000);
                    base as i64 * 2i64.pow(total_attempts as u32 - 1)
                }
                _ => job.backoff_delay.unwrap_or(5000) as i64,
            };

            let result = sqlx::query(
                r#"UPDATE pg_jobs SET state = 'waiting',
                    run_at = now() + ($2::bigint || ' milliseconds')::interval,
                    locked_by = NULL, locked_until = NULL,
                    last_error = $3 WHERE id = $1"#,
            )
            .bind(job.id)
            .bind(delay_ms)
            .bind(error)
            .execute(&self.pool)
            .await;

            if let Err(e) = result {
                tracing::error!("Failed to reschedule job {}: {}", job.id, e);
            }
        }
    }

    /// 恢复停滞的 job
    async fn recover_stalled_jobs(&self) -> Result<(), sqlx::Error> {
        let result = sqlx::query(
            r#"UPDATE pg_jobs SET state = 'waiting', locked_by = NULL, locked_until = NULL
            WHERE queue_name = $1
              AND state = 'active'
              AND locked_until < now()"#,
        )
        .bind(&self.queue_name)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() > 0 {
            tracing::warn!(
                "Recovered {} stalled jobs in {}",
                result.rows_affected(),
                self.queue_name
            );
        }
        Ok(())
    }

    /// 优雅关闭：等待所有正在执行的 job 完成
    async fn drain(&self, semaphore: &Semaphore) {
        tracing::info!(
            "Draining worker {}, waiting for {} active jobs...",
            self.queue_name,
            self.concurrency - semaphore.available_permits()
        );
        let _ = semaphore.acquire_many(self.concurrency as u32).await;
        tracing::info!("Worker {} drained", self.queue_name);
    }
}
