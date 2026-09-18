//! Queue consumer registration, shared by both binaries.
//!
//! `bin/server.rs` and `bin/worker.rs` used to carry identical copies of this
//! list. That is a correctness hazard, not just duplication: the queue names and
//! concurrency values have to agree, and two independently running consumers for
//! the same queue means each job competes for the same `pg_jobs` rows with the
//! full configured concurrency. Concurrency now comes from [`Config`] instead of
//! hard-coded literals, so `LANGFUSE_*_QUEUE_PROCESSING_CONCURRENCY` actually
//! does something.

use std::sync::Arc;

use langfuse_core::{Config, QueueName};
use langfuse_ingestion::IngestionProcessor;
use langfuse_queue::WorkerManager;
use langfuse_webhooks::{
    EntityChangeProcessor, MonitorProcessor, NotificationProcessor, WebhookProcessor,
};
use sqlx::PgPool;

/// Register every queue consumer with its configured concurrency.
pub fn register_all_queues(manager: &mut WorkerManager, pool: &PgPool, config: &Config) {
    // Ingestion Queue (核心 — 处理 trace/observation/score 事件)
    manager.register(
        QueueName::IngestionQueue.as_str(),
        Arc::new(IngestionProcessor::new(pool.clone())),
        config.ingestion_queue_concurrency,
    );

    manager.register(
        QueueName::WebhookQueue.as_str(),
        Arc::new(WebhookProcessor::new()),
        config.webhook_queue_concurrency,
    );

    manager.register(
        QueueName::NotificationQueue.as_str(),
        Arc::new(NotificationProcessor::new()),
        config.notification_queue_concurrency,
    );

    manager.register(
        QueueName::EntityChangeQueue.as_str(),
        Arc::new(EntityChangeProcessor::new()),
        config.entity_change_queue_concurrency,
    );

    manager.register(
        QueueName::MonitorQueue.as_str(),
        Arc::new(MonitorProcessor::new()),
        config.monitor_queue_concurrency,
    );

    tracing::info!(
        ingestion = config.ingestion_queue_concurrency,
        webhook = config.webhook_queue_concurrency,
        notification = config.notification_queue_concurrency,
        entity_change = config.entity_change_queue_concurrency,
        monitor = config.monitor_queue_concurrency,
        "queue consumers registered"
    );
}
