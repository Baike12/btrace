use crate::consumer::{JobProcessor, PgWorker};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

/// PgWorkerManager — 管理所有队列的 worker 生命周期
///
/// 对应原 `worker/src/pg-queue/PgWorkerManager.ts`
pub struct WorkerManager {
    pool: PgPool,
    workers: HashMap<String, Arc<PgWorker>>,
}

impl WorkerManager {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            workers: HashMap::new(),
        }
    }

    /// 注册一个队列的 worker（不启动）
    pub fn register(
        &mut self,
        queue_name: impl Into<String>,
        processor: Arc<dyn JobProcessor>,
        concurrency: usize,
    ) {
        let name = queue_name.into();
        let worker = Arc::new(PgWorker::new(
            name.clone(),
            self.pool.clone(),
            processor,
            concurrency,
        ));
        self.workers.insert(name.clone(), worker);
        tracing::info!("PgWorker registered: {} (concurrency={})", name, concurrency);
    }

    /// 启动所有已注册的 worker
    pub async fn run_all(self) {
        let handles: Vec<_> = self
            .workers
            .into_iter()
            .map(|(name, worker)| {
                tracing::info!("PgWorker starting: {}", name);
                tokio::spawn(async move {
                    worker.clone().run().await;
                })
            })
            .collect();

        // 等待所有 worker 结束（正常情况是 ctrl-c 后 drain）
        for handle in handles {
            if let Err(e) = handle.await {
                tracing::error!("Worker panicked: {}", e);
            }
        }
    }
}
