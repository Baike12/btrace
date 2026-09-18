use std::env;

/// 全局配置 - 从环境变量读取
///
/// `hostname` is the bind address, read from `LANGFUSE_BIND_ADDRESS` and
/// defaulting to `0.0.0.0`. It deliberately does **not** read `HOSTNAME`:
/// Docker sets that to the container id, so a container would try to bind to a
/// hostname that does not resolve and fail to start.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub salt: String,
    pub port: u16,
    pub hostname: String,
    pub session_max_age_minutes: i64,
    // Queue config
    pub ingestion_queue_concurrency: usize,
    pub webhook_queue_concurrency: usize,
    pub notification_queue_concurrency: usize,
    pub entity_change_queue_concurrency: usize,
    pub monitor_queue_concurrency: usize,
    pub queue_poll_interval_ms: u64,
    pub queue_stalled_interval_ms: u64,
    pub queue_max_attempts: i32,
    // Ingestion config
    pub ingestion_queue_delay_ms: i64,
    pub batch_writer_interval_ms: u64,
    pub batch_writer_batch_size: usize,
    pub batch_writer_max_retries: usize,
    // OTEL
    pub otel_trace_sampling_ratio: f64,
    pub otel_exporter_otlp_endpoint: Option<String>,
    // Email
    pub smtp_connection_url: Option<String>,
    pub email_from_address: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://baike@127.0.0.1:5432/lanfuse".to_string()),
            jwt_secret: env::var("JWT_SECRET")
                .unwrap_or_else(|_| "dev-secret-change-in-production".to_string()),
            salt: env::var("SALT").unwrap_or_else(|_| "dev-salt".to_string()),
            port: env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8080),
            hostname: env::var("LANGFUSE_BIND_ADDRESS")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "0.0.0.0".to_string()),
            session_max_age_minutes: env::var("AUTH_SESSION_MAX_AGE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(43200), // 30 days

            ingestion_queue_concurrency: env_var_usize("LANGFUSE_INGESTION_QUEUE_PROCESSING_CONCURRENCY", 20),
            webhook_queue_concurrency: env_var_usize("LANGFUSE_WEBHOOK_QUEUE_PROCESSING_CONCURRENCY", 5),
            notification_queue_concurrency: 5, // hardcoded in source
            entity_change_queue_concurrency: env_var_usize("LANGFUSE_ENTITY_CHANGE_QUEUE_PROCESSING_CONCURRENCY", 2),
            monitor_queue_concurrency: env_var_usize("LANGFUSE_MONITOR_QUEUE_PROCESSING_CONCURRENCY", 10),
            queue_poll_interval_ms: 500,
            queue_stalled_interval_ms: 120_000,
            queue_max_attempts: 6,

            ingestion_queue_delay_ms: env::var("LANGFUSE_INGESTION_QUEUE_DELAY_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5000),
            batch_writer_interval_ms: 500,
            batch_writer_batch_size: 1000,
            batch_writer_max_retries: 5,

            otel_trace_sampling_ratio: env::var("OTEL_TRACE_SAMPLING_RATIO")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            otel_exporter_otlp_endpoint: env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok(),

            smtp_connection_url: env::var("SMTP_CONNECTION_URL").ok(),
            email_from_address: env::var("EMAIL_FROM_ADDRESS").ok(),
        }
    }

    /// 需要写入 .env 的默认配置
    pub fn env_template() -> &'static str {
        r#"# Database
DATABASE_URL=postgres://baike@127.0.0.1:5432/lanfuse

# JWT secret (used for login/signup session tokens)
JWT_SECRET=dev-secret-change-in-production

# API Key salt (must match Node.js SALT)
SALT=dev-salt

# Server
PORT=8080
# Bind address for the API. Not HOSTNAME — Docker sets that to the container id.
LANGFUSE_BIND_ADDRESS=0.0.0.0

# OpenTelemetry
OTEL_TRACE_SAMPLING_RATIO=0
# OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318/v1/traces

# Email (optional)
# SMTP_CONNECTION_URL=smtp://localhost:1025
# EMAIL_FROM_ADDRESS=noreply@langfuse.local

# Queue concurrency overrides (optional)
# LANGFUSE_INGESTION_QUEUE_PROCESSING_CONCURRENCY=20
# LANGFUSE_WEBHOOK_QUEUE_PROCESSING_CONCURRENCY=5
# LANGFUSE_ENTITY_CHANGE_QUEUE_PROCESSING_CONCURRENCY=2
# LANGFUSE_MONITOR_QUEUE_PROCESSING_CONCURRENCY=10
"#
    }
}

fn env_var_usize(key: &str, default: usize) -> usize {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
