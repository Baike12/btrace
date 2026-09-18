# btrace 后端设计（Langfuse Rust 重构）

## 1. Context

当前 Langfuse 是 Node.js 全栈单体架构：Next.js (UI + 54 个 tRPC 路由器 + 72 个
REST API 路由) + Worker (5 个队列消费者)，本地开发内存 800-1200 MB（after OTEL
调优）。目标是将所有后端逻辑迁移到 Rust，Next.js 退化为纯前端渲染层，目标内存
250-350 MB（生产模式）。

**核心决策**（已与用户确认）：
- 前端：保留 Next.js 纯前端（SSR + 页面路由），移除所有 API routes 和 tRPC
- 后端：Rust + axum + sqlx + utoipa (OpenAPI)
- 数据库：继续使用同一个 PostgreSQL，Prisma 保留做 schema migration
- API 协议：REST + OpenAPI → openapi-typescript 生成前端 TypeScript 客户端
- 迁移策略：渐进式（Phase 1: Worker → Phase 2: API → Phase 3: 收口）

## 2. 目标架构

```
┌──────────────────────┐       ┌──────────────────────────┐       ┌──────────────┐
│  Next.js (纯前端)     │       │  Rust Backend (axum)      │       │  PostgreSQL  │
│                      │       │                          │       │              │
│  pages/              │  REST │  /api/auth/*             │  sqlx │  traces      │
│  components/         │──────→│  /api/traces/*           │──────→│  observations│
│  features/ (UI only) │  JSON │  /api/observations/*     │       │  scores      │
│  hooks/              │       │  /api/scores/*           │       │  datasets    │
│                      │       │  /api/public/ingestion   │       │  pg_jobs     │
│  SSR 渲染            │       │  /api/public/v2/*        │       │  api_keys    │
│  页面路由            │       │                          │       │  users       │
│  静态资源            │       │  + Queue Consumer (5个)   │       │  ...         │
│                      │       │  + Auth/RBAC             │       │              │
│  next start ~150MB   │       │  release build ~50-100MB │       │              │
└──────────────────────┘       └──────────────────────────┘       └──────────────┘
```

**关键变化**：
- 原 web `src/server/` (36 files) + `src/features/*/server/` (126 files) = 162 个 API 文件 → Rust
- 原 worker `src/` (146 files) → Rust
- 原 shared `src/server/` 中 repo/service/queue 代码 → Rust
- Next.js 只保留 `pages/` (路由)、`components/` (UI)、`features/*/` (前端部分)、`hooks/`、`utils/`

**数据流**：
1. SDK 提交事件 → `POST /api/public/ingestion` (Rust) → 入队 `pg_jobs` → 返回 207
2. Rust Queue Consumer 轮询 `pg_jobs` → IngestionService.mergeAndWrite → PgWriter batch write → PG
3. 前端读数据 → Next.js SSR `fetch()` → Rust REST API → sqlx → PG
4. 客户端交互 → 浏览器 `fetch()` → Rust REST API (带 JWT cookie)

## 3. Rust 项目结构

### 3.1 Cargo Workspace

```
langfuse-rs/
├── Cargo.toml                     # [workspace] members, shared dependencies
├── Cargo.lock
├── .env.example
├── README.md
│
├── crates/
│   ├── langfuse-core/             # 共享类型、domain models、error types
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs             # 模块导出
│   │       ├── types.rs           # Trace, Observation, Score 等 struct
│   │       ├── enums.rs           # ObservationType, ScoreSource, Role 等 enum
│   │       ├── errors.rs          # AppError, 错误类型 (thiserror)
│   │       └── config.rs          # 环境变量读取和验证
│   │
│   ├── langfuse-db/               # 数据库访问层
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── pool.rs            # PgPool 初始化 + 连接池配置
│   │       ├── repos/
│   │       │   ├── mod.rs
│   │       │   ├── traces.rs      # traces 表 CRUD + 聚合查询
│   │       │   ├── observations.rs
│   │       │   ├── scores.rs
│   │       │   ├── datasets.rs
│   │       │   ├── prompts.rs
│   │       │   ├── models.rs
│   │       │   ├── projects.rs
│   │       │   ├── users.rs
│   │       │   ├── api_keys.rs
│   │       │   ├── job_configs.rs
│   │       │   └── eval_templates.rs
│   │       ├── filter.rs          # FilterState → PG WHERE clause 翻译
│   │       ├── order_by.rs        # OrderBy → PG ORDER BY 翻译
│   │       └── pagination.rs      # 游标分页工具
│   │
│   ├── langfuse-queue/            # PG 原生的队列系统
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── producer.rs        # PgQueue: add(), 入队 INSERT
│   │       ├── consumer.rs        # PgWorker: poll(), FOR UPDATE SKIP LOCKED
│   │       ├── manager.rs         # PgWorkerManager: 注册 + 生命周期
│   │       ├── dedup.rs           # pg_seen_events: checkAndMarkSeen
│   │       └── retry.rs           # 退避策略 (exponential, fixed)
│   │
│   ├── langfuse-ingestion/        # Ingestion 核心业务逻辑
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── service.rs         # IngestionService::merge_and_write()
│   │       ├── trace_processor.rs # processTraceEventList
│   │       ├── observation_processor.rs  # processObservationEventList
│   │       ├── score_processor.rs       # processScoreEventList
│   │       ├── dataset_processor.rs     # processDatasetRunItemEventList
│   │       ├── merger.rs          # mergeRecords 逻辑
│   │       ├── validator.rs       # Zod 等价：event schema 验证
│   │       └── writer.rs          # PgWriter 等价：批量 INSERT ON CONFLICT
│   │
│   ├── langfuse-api/              # axum HTTP server
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── app.rs             # Router 顶层组装，CORS，中间件
│   │       ├── middleware/
│   │       │   ├── mod.rs
│   │       │   ├── auth.rs        # JWT session 验证中间件
│   │       │   ├── api_key.rs     # API Key 验证中间件
│   │       │   ├── rbac.rs        # 角色 → scope 权限检查
│   │       │   ├── rate_limit.rs  # 限速
│   │       │   └── tracing.rs     # 请求日志 + tracing span
│   │       ├── routes/
│   │       │   ├── mod.rs
│   │       │   ├── auth.rs        # /api/auth/login, signup, session
│   │       │   ├── traces.rs      # /api/traces/*
│   │       │   ├── observations.rs
│   │       │   ├── scores.rs
│   │       │   ├── datasets.rs
│   │       │   ├── prompts.rs
│   │       │   ├── sessions.rs
│   │       │   ├── evals.rs
│   │       │   ├── models.rs
│   │       │   ├── projects.rs
│   │       │   ├── organizations.rs
│   │       │   ├── users.rs
│   │       │   ├── api_keys.rs
│   │       │   ├── dashboards.rs
│   │       │   ├── comments.rs
│   │       │   ├── media.rs
│   │       │   ├── automations.rs
│   │       │   └── monitors.rs
│   │       ├── public_api/        # /api/public/* 对应原 pages/api/public/*
│   │       │   ├── mod.rs
│   │       │   ├── ingestion.rs   # POST /api/public/ingestion
│   │       │   ├── v2/
│   │       │   │   ├── mod.rs
│   │       │   │   ├── traces.rs
│   │       │   │   ├── observations.rs
│   │       │   │   ├── scores.rs
│   │       │   │   ├── datasets.rs
│   │       │   │   ├── prompts.rs
│   │       │   │   └── metrics.rs
│   │       │   ├── otel/          # OTLP trace/metric ingestion
│   │       │   └── ...
│   │       ├── openapi.rs         # utoipa 配置，生成 openapi.json
│   │       └── response.rs        # 统一 API 响应格式 { data, meta, error }
│   │
│   ├── langfuse-auth/             # 认证和 RBAC
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── jwt.rs             # JWT 签发 + 验证 (jsonwebtoken)
│   │       ├── api_key.rs         # API Key 验证 (bcrypt + SHA-256 fast hash)
│   │       ├── session.rs         # Session 结构定义
│   │       ├── rbac.rs            # Role → Scope 映射，权限检查函数
│   │       └── password.rs        # 密码哈希 (argon2 或 bcrypt)
│   │
│   └── langfuse-webhooks/         # Webhook/通知 处理
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── processor.rs       # Webhook 队列处理器
│           ├── slack.rs           # Slack Block Kit 消息构建
│           ├── github.rs          # GitHub repository_dispatch
│           ├── email.rs           # 邮件发送 (SMTP / SES)
│           └── notification.rs    # 通知路由
│
├── migrations/                    # sqlx 生成的 migration 文件
│   └── (Prisma 已管理，此目录仅在 Phase 3 考虑使用)
│
├── bin/
│   ├── server.rs                  # API server 入口 (Phase 2)
│   └── worker.rs                  # Queue consumer 入口 (Phase 1)
│
└── scripts/
    ├── generate-openapi.sh        # 生成 OpenAPI JSON → TypeScript client
    └── dev.sh                     # 本地开发启动脚本
```

### 3.2 Crate 依赖关系

```
langfuse-core       ← 所有 crate 依赖的基础类型
    ↑
langfuse-db         ← 依赖 core 的 types/enums/errors
    ↑
langfuse-queue      ← 依赖 db (连接池) + core
    ↑
langfuse-ingestion  ← 依赖 db + queue + core
    ↑
langfuse-auth       ← 依赖 db + core
    ↑
langfuse-webhooks   ← 依赖 db + core
    ↑
langfuse-api        ← 依赖所有 crate，组装为 HTTP server
    ↑
bin/server.rs       ← 入口
bin/worker.rs       ← 入口 (仅依赖 queue + ingestion + webhooks + core + db)
```

### 3.3 关键依赖（Cargo.toml）

```toml
# langfuse-rs/Cargo.toml (workspace)
[workspace.dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }
# HTTP framework
axum = "0.8"
# Database
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres", "uuid", "chrono"] }
# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
# Validation
validator = { version = "0.20", features = ["derive"] }   # Zod 等价
# JWT
jsonwebtoken = "9"
# Password hashing
bcrypt = "0.16"
argon2 = "0.5"
# SHA-256 (API key fast hash)
sha2 = "0.10"
hex = "0.4"
# UUID
uuid = { version = "1", features = ["v4", "v7", "serde"] }
# Time
chrono = { version = "0.4", features = ["serde"] }
# OpenAPI
utoipa = { version = "5", features = ["axum_extras", "chrono"] }
utoipa-swagger-ui = { version = "8", features = ["axum"] }
# Error handling
thiserror = "2"
anyhow = "1"
# Tracing
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
tracing-opentelemetry = "0.29"
opentelemetry = "0.28"
opentelemetry_sdk = { version = "0.28", features = ["rt-tokio"] }
opentelemetry-otlp = { version = "0.28", features = ["tonic"] }
# HTTP client (webhook calls)
reqwest = { version = "0.12", features = ["json"] }
# Config
dotenvy = "0.15"
# Email
lettre = { version = "0.11", features = ["tokio1", "tokio1-native-tls"] }
# Rate limiting
governor = "0.7"
# Testing
sqlx-cli = "0.8"
```

## 4. 数据库设计

### 4.1 Migration 策略

**保留 Prisma 管理 schema**，不引入 sqlx migrate。原因：
- 69 个 model 的 migration 历史用 Prisma 管理成熟稳定
- 避免两套 migration 系统并行导致的不一致
- sqlx 只需要编译期 SQL 检查，不需要管理 migration

```rust
// langfuse-db/src/pool.rs
use sqlx::postgres::{PgPool, PgPoolOptions};

pub async fn init_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(20)          // 生产模式，不是 num_cpus*2+1=46
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(300))
        .max_lifetime(Duration::from_secs(1800))
        .connect(database_url)
        .await
        .expect("Failed to create PG pool")
}
```

**sqlx 编译期 SQL 检查**：在 CI 和本地开发时，sqlx 需要一个可连接的数据库来验证 SQL
语法正确性。这通过 `sqlx::query_as!` 宏在编译期完成。

### 4.2 Repository 模式

```rust
// langfuse-db/src/repos/traces.rs
use sqlx::PgPool;
use langfuse_core::types::Trace;

pub struct TraceRepo {
    pool: PgPool,
}

impl TraceRepo {
    pub fn new(pool: PgPool) -> Self { Self { pool } }

    /// 查询 trace，返回最新版本 (DISTINCT ON id, project_id)
    pub async fn find_by_id(&self, id: &str, project_id: &str) -> Result<Option<Trace>> {
        sqlx::query_as!(
            Trace,
            r#"SELECT DISTINCT ON (id, project_id) *
               FROM traces
               WHERE id = $1 AND project_id = $2
               ORDER BY id, project_id, updated_at DESC"#,
            id, project_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// 列出 traces（带过滤 + 分页 + 排序）
    pub async fn list(
        &self,
        project_id: &str,
        filter: &FilterState,
        order_by: &OrderBy,
        cursor: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Trace>> {
        // 动态构建 SQL：FilterState → WHERE clause, OrderBy → ORDER BY
        // 使用 sqlx::QueryBuilder 安全拼接
        todo!()
    }

    /// 观测聚合 (CTE JOIN)
    pub async fn list_with_observation_agg(
        &self,
        project_id: &str,
        time_window: (DateTime<Utc>, DateTime<Utc>),
        filter: &FilterState,
        limit: i64,
    ) -> Result<Vec<TraceWithStats>> {
        // 对应原 traces.ts 的 CTE 查询
        todo!()
    }
}
```

### 4.3 Filter → SQL 翻译

对应原 `packages/shared/src/server/filterToPrisma.ts` + `queries/pg-sql/pg-filter.ts`：

```rust
// langfuse-db/src/filter.rs
use sqlx::QueryBuilder;

pub enum FilterOperator {
    Eq, NotEq, Contains, NotContains, StartsWith, EndsWith,
    Gt, Gte, Lt, Lte,
    AnyOf, NoneOf, AllOf,
    IsNull, IsNotNull,
}

pub struct FilterCondition {
    pub column: String,
    pub operator: FilterOperator,
    pub value: serde_json::Value,
}

pub fn apply_filters(
    query: &mut QueryBuilder<'_, sqlx::Postgres>,
    filters: &[FilterCondition],
    param_offset: &mut usize,
) {
    for filter in filters {
        match filter.operator {
            FilterOperator::Eq => {
                query.push(format!(" AND {} = ", filter.column));
                query.push_bind(filter.value.as_str().unwrap());
            }
            FilterOperator::AnyOf => {
                query.push(format!(" AND {} = ANY(", filter.column));
                query.push_bind(filter.value.as_array().unwrap()
                    .iter().map(|v| v.as_str().unwrap().to_string())
                    .collect::<Vec<_>>());
                query.push("::text[])");
            }
            // ... 其他操作符
        }
    }
}
```

### 4.4 类型映射

| Prisma Type | Rust Type | PG Column | sqlx Type |
|-------------|-----------|-----------|-----------|
| `String` | `String` | `TEXT` | `String` |
| `Int` | `i32` | `INTEGER` | `i32` |
| `Float` | `f64` | `DOUBLE PRECISION` | `f64` |
| `Boolean` | `bool` | `BOOLEAN` | `bool` |
| `DateTime` | `chrono::DateTime<Utc>` | `TIMESTAMPTZ` | `DateTime<Utc>` |
| `Json` | `serde_json::Value` | `JSONB` | `JsonValue` |
| `Decimal` | `rust_decimal::Decimal` | `DECIMAL` | `Decimal` |
| `String[]` | `Vec<String>` | `TEXT[]` | `Vec<String>` |
| `BigInt` | `i64` | `BIGINT` | `i64` |
| Enum | Rust enum (String) | PG ENUM | 需要 CAST |

**Domain Types 示例**：

```rust
// langfuse-core/src/types.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Trace {
    pub id: String,
    pub project_id: String,
    pub name: Option<String>,
    pub user_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub release: Option<String>,
    pub version: Option<String>,
    pub environment: Option<String>,
    pub public: bool,
    pub bookmarked: bool,
    pub tags: Vec<String>,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub session_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

## 5. 队列系统设计（Phase 1 核心）

### 5.1 设计原则

- 与现有 `pg_jobs` 表完全兼容，新旧 worker 可以共存
- 同样使用 `SELECT ... FOR UPDATE SKIP LOCKED` 模式
- Rust 的 tokio 任务模型天然适合高并发队列消费

### 5.2 PgWorker 实现

```rust
// langfuse-queue/src/consumer.rs
use sqlx::PgPool;
use tokio::sync::Semaphore;
use std::sync::Arc;
use std::time::Duration;

pub struct PgWorker<C> {
    queue_name: String,
    pool: PgPool,
    processor: Arc<dyn JobProcessor<Context = C>>,
    context: C,
    concurrency: usize,          // 最大并发数
    poll_interval: Duration,     // 轮询间隔 (500ms)
    stalled_interval: Duration,  // 停滞恢复间隔 (120s)
    max_attempts: i32,          // 最大重试次数 (默认 5)
    rate_limiter: Option<RateLimiter>,
    worker_id: String,           // UUID，用于 locked_by
}

impl<C: Clone + Send + Sync + 'static> PgWorker<C> {
    /// 核心轮询循环
    async fn poll(&self, semaphore: &Semaphore) -> Result<Vec<Job>> {
        // 1. 计算可领取数量
        let capacity = self.concurrency - semaphore.available_permits();
        let take = capacity.min(10); // 每次最多 10 个

        if take == 0 { return Ok(vec![]); }

        // 2. 原子认领 (FOR UPDATE SKIP LOCKED)
        let jobs = sqlx::query_as!(
            Job,
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
            self.queue_name,
            take as i64,
            self.worker_id,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(jobs)
    }

    /// 启动 worker（主循环）
    pub async fn run(self: Arc<Self>) {
        let semaphore = Arc::new(Semaphore::new(self.concurrency));

        // 主轮询循环
        let poll_handle = {
            let this = self.clone();
            let sem = semaphore.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(this.poll_interval);
                loop {
                    interval.tick().await;
                    let jobs = this.poll(&sem).await.unwrap_or_default();
                    for job in jobs {
                        let permit = sem.clone().acquire_owned().await.unwrap();
                        let this = this.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            this.execute_job(job).await;
                        });
                    }
                }
            })
        };

        // 停滞恢复循环
        let recovery_handle = {
            let this = self.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(this.stalled_interval);
                loop {
                    interval.tick().await;
                    if let Err(e) = this.recover_stalled_jobs().await {
                        tracing::error!("Stalled recovery failed: {}", e);
                    }
                }
            })
        };

        tokio::select! {
            _ = poll_handle => {},
            _ = recovery_handle => {},
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Worker {} shutting down", self.queue_name);
                self.graceful_shutdown(&semaphore).await;
            }
        }
    }

    /// 执行单个 job
    async fn execute_job(&self, job: Job) {
        let result = self.processor.process(&self.context, &job).await;
        match result {
            Ok(()) => {
                // 标记完成
                sqlx::query!(
                    "UPDATE pg_jobs SET state = 'completed', finished_at = now()
                     WHERE id = $1",
                    job.id
                )
                .execute(&self.pool)
                .await
                .ok();
            }
            Err(e) => {
                self.handle_failure(&job, &e).await;
            }
        }
    }

    /// 失败重试逻辑
    async fn handle_failure(&self, job: &Job, error: &str) {
        if job.attempts >= self.max_attempts {
            // 永久失败
            sqlx::query!(
                "UPDATE pg_jobs SET state = 'failed', finished_at = now(),
                 last_error = $2 WHERE id = $1",
                job.id, error
            )
            .execute(&self.pool)
            .await
            .ok();
        } else {
            // 指数退避重试
            let delay_ms = match job.backoff_type.as_deref() {
                Some("exponential") => {
                    job.backoff_delay * 2i32.pow(job.attempts as u32 - 1)
                }
                _ => job.backoff_delay,
            };
            sqlx::query!(
                "UPDATE pg_jobs SET state = 'waiting',
                 run_at = now() + $2 * interval '1 millisecond',
                 locked_by = NULL, locked_until = NULL,
                 last_error = $3 WHERE id = $1",
                job.id, delay_ms as i64, error
            )
            .execute(&self.pool)
            .await
            .ok();
        }
    }
}
```

### 5.3 PgQueue（生产者）

```rust
// langfuse-queue/src/producer.rs
pub struct PgQueue {
    pool: PgPool,
    queue_name: String,
}

impl PgQueue {
    pub async fn add(&self, name: &str, payload: &serde_json::Value,
                     opts: &QueueOptions) -> Result<()> {
        sqlx::query!(
            r#"INSERT INTO pg_jobs (
                job_id, queue_name, payload, state, run_at,
                max_attempts, backoff_type, backoff_delay
            ) VALUES ($1, $2, $3, 'waiting', now() + $4 * interval '1 millisecond',
                      $5, $6, $7)"#,
            uuid::Uuid::new_v4(),
            self.queue_name,
            payload,
            opts.delay_ms as i64,
            opts.max_attempts,
            opts.backoff_type.as_deref(),
            opts.backoff_delay_ms,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
```

### 5.4 队列注册（Phase 1）

```rust
// bin/worker.rs — Phase 1 入口
use langfuse_db::pool::init_pool;
use langfuse_queue::manager::WorkerManager;
use langfuse_ingestion::service::IngestionProcessor;
use langfuse_webhooks::processor::WebhookProcessor;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,langfuse=debug")
        .init();

    let pool = init_pool(&std::env::var("DATABASE_URL")?).await;
    let mut manager = WorkerManager::new();

    // 启动 5 个队列消费者
    manager.register(
        "ingestion-queue",
        IngestionProcessor::new(pool.clone()),
        20,  // concurrency
    );
    manager.register(
        "webhook-queue",
        WebhookProcessor::new(pool.clone()),
        5,
    );
    manager.register(
        "notification-queue",
        NotificationProcessor::new(pool.clone()),
        5,
    );
    manager.register(
        "entity-change-queue",
        EntityChangeProcessor::new(pool.clone()),
        2,
    );
    manager.register(
        "monitor-queue",
        MonitorProcessor::new(pool.clone()),
        10,
    );

    manager.run_all().await;  // spawn all workers, wait for ctrl-c
    Ok(())
}
```

**Phase 1 验证检查点**：
1. `cargo run --bin worker` 启动后，能在日志中看到 "PgWorker registered: ingestion-queue"
2. 向 `pg_jobs` 手动插入一条 ingestion job，worker 能消费并写入 traces/observations/scores
3. 与 Node.js web 共存：web 正常接入 SDK 事件 → enqueue → Rust worker 消费 → 数据写入 PG
4. 内存占用：Rust worker < 50MB

## 6. Ingestion 服务设计（Phase 1 核心）

这是整个系统最核心的业务逻辑，对应原 `worker/src/services/IngestionService/index.ts` (1648 行)。

### 6.1 数据流

```
PgWorker poll → 认领 ingestion job
  |
  v
IngestionProcessor::process(job)
  |
  ├── 从 job.payload 读取 events[]
  ├── checkAndMarkSeen(project_id, type, event_id, key) → pg_seen_events 去重
  ├── 按 entityType 分组
  │     ├── "trace" → process_trace_events()
  │     ├── "observation" → process_observation_events()
  │     ├── "score" → process_score_events()
  │     └── "dataset_run_item" → process_dataset_events()
  │
  ├── 每个分组:
  │     ├── sort events (create first, update last)
  │     ├── map events → record structs
  │     ├── merge_records(existing, incoming)  // PG-only: existing always None
  │     ├── enrich (prompt lookup, cost calc, session upsert)
  │     └── BatchWriter::add(table, record)
  │
  └── job 完成 (PgWorker 标记 completed)
```

### 6.2 BatchWriter（PgWriter 等价）

```rust
// langfuse-ingestion/src/writer.rs
use tokio::sync::mpsc;
use std::collections::HashMap;
use std::time::Duration;

pub struct BatchWriter {
    pool: PgPool,
    queues: Arc<RwLock<HashMap<TableName, Vec<serde_json::Value>>>>,
    batch_size: usize,     // 1000
    write_interval: Duration, // 500ms
    max_retries: usize,    // 5
}

impl BatchWriter {
    pub fn new(pool: PgPool) -> Self {
        let writer = Self {
            pool,
            queues: Arc::new(RwLock::new(HashMap::new())),
            batch_size: 1000,
            write_interval: Duration::from_millis(500),
            max_retries: 5,
        };
        writer.start_flush_timer();
        writer
    }

    /// 添加一条记录到批量队列
    pub async fn add(&self, table: TableName, record: serde_json::Value) {
        let mut queues = self.queues.write().await;
        let batch = queues.entry(table).or_default();
        batch.push(record);
        if batch.len() >= self.batch_size {
            drop(queues);  // 释放锁
            self.flush(table).await;
        }
    }

    /// 定时 flush
    fn start_flush_timer(&self) {
        let queues = self.queues.clone();
        let pool = self.pool.clone();
        let max_retries = self.max_retries;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;
                let tables: Vec<TableName> = queues.read().await.keys().cloned().collect();
                for table in tables {
                    flush_table(&pool, &queues, table, max_retries).await;
                }
            }
        });
    }

    /// 批量 INSERT ... ON CONFLICT DO UPDATE
    async fn flush_table(&self, table: TableName) -> Result<()> {
        let mut queues = self.queues.write().await;
        let batch: Vec<_> = queues.entry(table).or_default()
            .drain(..std::cmp::min(self.batch_size, queues[&table].len()))
            .collect();
        drop(queues);

        if batch.is_empty() { return Ok(()); }

        let result = self.write_to_pg(table, &batch).await;
        match result {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::error!("Batch write failed: {}", e);
                // 重试逻辑 (最多 max_retries 次)
                todo!()
            }
        }
    }

    /// 构建 INSERT ... ON CONFLICT DO UPDATE
    async fn write_to_pg(&self, table: TableName, records: &[serde_json::Value]) -> Result<()> {
        let columns = Self::column_list(table);  // 按 PG_TABLE_COLUMNS 过滤
        let conflict_target = match table {
            TableName::Traces => "(id) DO UPDATE",
            _ => "(id, project_id) DO UPDATE",
        };

        // 构建参数化 SQL
        let mut query = String::new();
        query.push_str(&format!(
            "INSERT INTO {} ({}) VALUES ", table.as_str(), columns.join(", ")
        ));

        let values: Vec<String> = (0..records.len())
            .map(|i| {
                let offset = i * columns.len();
                columns.iter().enumerate()
                    .map(|(j, _)| format!("${}", offset + j + 1))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .map(|row| format!("({})", row))
            .collect();
        query.push_str(&values.join(", "));
        query.push_str(&format!(" ON CONFLICT {} SET ...", conflict_target));

        // 参数化执行 (安全，无 SQL 注入)
        sqlx::query(&query)
            // ... 绑定所有参数
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
```

### 6.3 Merge 逻辑

```rust
// langfuse-ingestion/src/merger.rs

/// 合并 trace 事件列表为最终记录
/// 对应 processTraceEventList 中的 mergeTraceRecords
pub fn merge_trace_records(
    events: &[TraceEvent],
    existing: Option<&Trace>,  // PG-only: always None
) -> TraceRecord {
    let mut record = TraceRecord::default();

    let immutable_keys: HashSet<&str> = [
        "id", "project_id", "timestamp", "created_at", "environment"
    ].iter().copied().collect();

    for event in events.iter().sorted_by_key(|e| e.timestamp) {
        // 叠加每个事件的字段，保护不可变字段
        apply_event_fields(&mut record, event, &immutable_keys);
    }

    // 最后一个有 input/output 的事件决定最终值
    for event in events.iter().rev() {
        if event.input.is_some() && record.input.is_none() {
            record.input = event.input.clone();
        }
        if event.output.is_some() && record.output.is_none() {
            record.output = event.output.clone();
        }
    }

    record
}
```

## 7. API 层设计（Phase 2）

### 7.1 Router 结构

```rust
// langfuse-api/src/app.rs
use axum::{Router, middleware};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

pub fn create_app(pool: PgPool, jwt_secret: &[u8]) -> Router {
    let auth_middleware = middleware::from_fn_with_state(
        pool.clone(), auth::verify_session
    );
    let api_key_middleware = middleware::from_fn_with_state(
        pool.clone(), auth::verify_api_key
    );

    let public_router = Router::new()
        .route("/api/public/ingestion", post(public_api::ingestion::handler))
        .route("/api/public/health", get(public_api::health::handler))
        .nest("/api/public/v2", public_api::v2::router())
        // ... 其他公开路由
        .layer(api_key_middleware);

    let private_router = Router::new()
        .nest("/api/traces", routes::traces::router())
        .nest("/api/observations", routes::observations::router())
        .nest("/api/scores", routes::scores::router())
        .nest("/api/sessions", routes::sessions::router())
        .nest("/api/datasets", routes::datasets::router())
        .nest("/api/prompts", routes::prompts::router())
        .nest("/api/models", routes::models::router())
        .nest("/api/evals", routes::evals::router())
        .nest("/api/projects", routes::projects::router())
        .nest("/api/organizations", routes::organizations::router())
        .nest("/api/dashboards", routes::dashboards::router())
        .nest("/api/comments", routes::comments::router())
        .nest("/api/media", routes::media::router())
        .nest("/api/automations", routes::automations::router())
        .nest("/api/monitors", routes::monitors::router())
        .nest("/api/auth", routes::auth::router())
        // ... 对应原 54 个 tRPC 路由器
        .layer(auth_middleware);

    Router::new()
        .merge(public_router)
        .merge(private_router)
        .merge(SwaggerUi::new("/api/docs").url("/api/openapi.json", ApiDoc::openapi()))
        .layer(tower_http::cors::CorsLayer::permissive())
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(pool)
}
```

### 7.2 资源路由示例

```rust
// langfuse-api/src/routes/traces.rs
use axum::{Router, extract::{State, Path, Query}, Json};
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct ListTracesQuery {
    pub project_id: String,
    pub filter: Option<Vec<FilterCondition>>,
    pub order_by: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct ListTracesResponse {
    pub data: Vec<Trace>,
    pub meta: PaginationMeta,
}

/// 列出 traces
#[utoipa::path(
    get,
    path = "/api/traces",
    params = ListTracesQuery,
    responses(
        (status = 200, body = ListTracesResponse),
    )
)]
pub async fn list_traces(
    State(pool): State<PgPool>,
    Query(params): Query<ListTracesQuery>,
    session: Session,  // 从 auth middleware 提取
) -> Result<Json<ListTracesResponse>, AppError> {
    // RBAC 检查
    rbac::check_scope(&session, &params.project_id, "traces:read")?;

    let repo = TraceRepo::new(pool);
    let (traces, meta) = repo
        .list_with_observation_agg(
            &params.project_id,
            &params.filter.unwrap_or_default(),
            &params.order_by.unwrap_or_default(),
            params.cursor.as_deref(),
            params.limit.unwrap_or(50),
        )
        .await?;

    Ok(Json(ListTracesResponse { data: traces, meta }))
}

/// 获取单个 trace（含 observations tree）
#[utoipa::path(
    get,
    path = "/api/traces/{trace_id}",
)]
pub async fn get_trace(
    State(pool): State<PgPool>,
    Path(trace_id): Path<String>,
    Query(project_id): Query<String>,
    session: Session,
) -> Result<Json<TraceDetail>, AppError> {
    rbac::check_scope(&session, &project_id, "traces:read")?;

    let repo = TraceRepo::new(pool);
    let trace = repo.find_by_id(&trace_id, &project_id).await?
        .ok_or(AppError::NotFound("Trace not found".into()))?;

    let obs_repo = ObservationRepo::new(pool.clone());
    let observations = obs_repo
        .find_by_trace_id(&trace_id, &project_id)
        .await?;

    // 构建 observation tree (递归)
    let tree = build_observation_tree(observations);

    Ok(Json(TraceDetail { trace, observations: tree }))
}

pub fn router() -> Router {
    Router::new()
        .route("/", get(list_traces))
        .route("/{trace_id}", get(get_trace))
}
```

### 7.3 OpenAPI 集成

```rust
// langfuse-api/src/openapi.rs
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "Langfuse API", version = "1.0.0"),
    paths(
        routes::traces::list_traces,
        routes::traces::get_trace,
        routes::observations::list_observations,
        // ... 注册所有 handler
    ),
    components(schemas(
        Trace, Observation, Score, Dataset, Prompt,
        PaginationMeta, FilterCondition,
        // ... 注册所有 schema
    ))
)]
pub struct ApiDoc;
```

**前端类型生成**：

```bash
# scripts/generate-openapi.sh
cargo run --bin server &  # 启动 server
sleep 2
curl http://localhost:8080/api/openapi.json -o openapi.json
npx openapi-typescript openapi.json -o web/src/api/client.ts
kill %1
```

## 8. 认证系统设计

### 8.1 JWT Session（替代 NextAuth）

```rust
// langfuse-auth/src/jwt.rs
use jsonwebtoken::{encode, decode, Header, Validation, EncodingKey, DecodingKey};
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,                    // user_id
    pub exp: usize,                     // expiry
    pub iat: usize,                     // issued at
    pub orgs: Vec<OrgMembership>,      // 嵌入 session 的组织/项目信息
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OrgMembership {
    pub org_id: String,
    pub org_name: String,
    pub org_role: Role,
    pub projects: Vec<ProjectMembership>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectMembership {
    pub project_id: String,
    pub project_name: String,
    pub project_role: Role,
}

/// 签发 JWT token
pub fn create_token(user: &User, memberships: &[OrgMembership],
                    secret: &[u8], max_age_minutes: i64) -> Result<String> {
    let now = chrono::Utc::now();
    let claims = Claims {
        sub: user.id.clone(),
        exp: (now + chrono::Duration::minutes(max_age_minutes)).timestamp() as usize,
        iat: now.timestamp() as usize,
        orgs: memberships.to_vec(),
    };
    encode(&Header::default(), &claims, &EncodingKey::from_secret(secret))
        .map_err(Into::into)
}

/// 验证 JWT token
pub fn verify_token(token: &str, secret: &[u8]) -> Result<Claims> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret),
        &Validation::default(),
    )?;
    Ok(data.claims)
}
```

### 8.2 Auth 中间件

`middleware/auth.rs` 校验会话并把 `Session` 注入 request extensions。凭据从
`langfuse_session` cookie 或 `Authorization: Bearer <jwt>` 取，两者都走
`langfuse_auth::jwt::verify_token`，密钥是 `AppState.jwt_secret`（来自 `JWT_SECRET`）。

```rust
// langfuse-api/src/middleware/auth.rs（节选）
pub async fn verify_session(
    State(state): State<AppState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Response> {
    let Some(token) = session_token(&req) else {
        return Err(unauthorized("No session. Sign in first."));
    };

    let claims = verify_token(&token, &state.jwt_secret)
        .map_err(|e| { tracing::debug!(error = %e, "session token rejected"); unauthorized("Invalid or expired session. Sign in again.") })?;

    req.extensions_mut().insert(Session::from(claims));
    Ok(next.run(req).await)
}
```

失败返回公开 API 同款 `{message, error}` 体。**没有回退身份**：缺凭据、伪造签名、过期
一律 401，不存在"落到某个默认用户"的分支。

**中间件只回答"你是谁"，不回答"你能看哪个项目"。** `project_id` 在几乎所有私有路由上
都是查询参数，因此每个 handler 还必须调用：

```rust
// 会话用户是否为该项目所属组织的成员
pub async fn require_project_access(
    pool: &PgPool, session: &Session, project_id: &str,
) -> Result<(), (StatusCode, String)>   // 不属于 → 404
```

返回 404 而非 403：403 等于确认该项目存在，任何已登录用户都能据此枚举 project id。

### 8.3 RBAC 检查

本分支没有细粒度 scope 系统。`Session` 只带 `user_id`/`email`/`name`，授权是**一个判断**：
会话用户是否属于拥有该项目的组织。

```rust
// langfuse-auth/src/rbac.rs
pub async fn user_can_access_project(
    pool: &PgPool, user_id: &str, project_id: &str,
) -> Result<bool> {
    sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS (
               SELECT 1 FROM projects p
               JOIN organization_memberships om ON om.org_id = p.org_id
               WHERE p.id = $1 AND om.user_id = $2
           )"#,
    ).bind(project_id).bind(user_id).fetch_one(pool).await.map_err(Into::into)
}
```

依据是**组织成员关系**，不是 `project_memberships`：后者只细化了用户在该项目内的角色
（OWNER/MEMBER/VIEWER），不改变项目可见性。两者口径必须一致——`/api/auth/session` 用
同一套 memberships 渲染侧边栏的项目列表，判定更严就会出现「侧边栏看得见、点进去 404」。

> 早先这里有两个恒返回 `Ok(())` 的函数（`rbac::check_project_scope` /
> `routes::traces::verify_project_access`）。它们不只是没实现，还会让阅读代码的人以为
> 授权已经做过了。已删除，不留占位。

细粒度角色（谁能删项目、谁只能看）属于未实现范围；当前所有组织成员权限相同。

### 8.4 API Key 验证

`/api/public/*` 用 API Key（`Authorization: Basic base64(pk:sk)`），与 8.2 的会话 cookie
是**两套互不相通的凭据**。两阶段验证：先比 fast hash，再回落到 bcrypt。

```rust
// langfuse-core/src/api_key_hash.rs —— 生成与校验共用同一份实现
pub fn fast_hash_secret_key(secret_key: &str, salt: &str) -> String {
    let inner = hex::encode(Sha256::digest(secret_key.as_bytes()));   // 内层，先 hex
    hex::encode(Sha256::digest(format!("{inner}{salt}").as_bytes()))  // 外层，再拼 salt
}
```

> **内层是 `Sha256(secret_key)` 的十六进制，不是裸字节。** 写成
> `Sha256(secret_key + salt)`（本文件早先的草图就是这样）会算出完全不同的值，所有 key
> 都验不过。这也是为什么创建路径（`bootstrap.rs`、`routes/api_keys.rs`）必须调用同一个
> 函数而不是各写一份。

校验流程（`langfuse-auth/src/api_key.rs`）：

```text
pk ──▶ SELECT id, hashed_secret_key, fast_hashed_secret_key,
       scope, project_id, organization_id, expires_at
       FROM api_keys WHERE public_key = $1
  │
  ├─ expires_at < now() → 401 "API key expired"
  ├─ fast_hashed_secret_key == fast_hash(sk)  → 通过，更新 last_used_at
  └─ 否则 bcrypt::verify(sk, hashed_secret_key)
        └─ 通过 → 顺带把 fast hash 写回（老 key 自动升级），更新 last_used_at
```

返回的 `ApiKeyScope` 只带 `project_id` / `org_id` / `scope` / `api_key_id`，中间件把它
注入 request extensions；公开 API 的每个 handler 用 `scope.project_id` 定位数据，
**不信任请求里的任何 project 标识**。

### 8.5 Next.js 前端 Auth 适配

前端不走生成的 OpenAPI 客户端，而是一个无类型代理 + 同源 fetch：

- `web/src/utils/api.ts` —— 所有 tRPC 形状的调用点都经它发到 Next.js 同源
  `/api/*`，由 `next.config.mjs` 的 rewrites 在**服务端**转发到 Rust（`RUST_API_URL`）。
  浏览器因此从不直连 :8010，也就不需要 CORS。
- 代价是类型丢失：tRPC 换成这层代理后约 800 个调用点没有类型，`next build` 的类型检查
  被 `NEXT_IGNORE_BUILD_ERRORS=true` 跳过（见 §13.9）。
- `web/src/hooks/useAuth.ts` —— 会话。`signIn` 打 `POST /api/auth/login`，浏览器自动带
  上 `Set-Cookie` 的 `HttpOnly` cookie；后续请求靠 `credentials: "same-origin"` 携带。
  没有硬编码回退会话：取不到就是未登录，交给 `useAuthGuard` 跳 `/auth/sign-in`。

## 9. Phase-by-Phase 迁移计划

### Phase 1: Worker 迁移（预计 3-4 周）

**目标**：Rust worker 完全替代 Node.js worker，与 Node.js web 共存。

**改动范围**：
- 新建 `langfuse-rs/` 整个 Rust workspace
- 实现 `langfuse-core` (types, enums, errors, config)
- 实现 `langfuse-db` (pool, repos: traces, observations, scores, datasets, events)
- 实现 `langfuse-queue` (PgWorker, PgQueue, dedup, retry)
- 实现 `langfuse-ingestion` (IngestionService, BatchWriter, merger, validator)
- 实现 `langfuse-webhooks` (WebhookProcessor, Slack, email)
- 实现 `bin/worker.rs` 入口

**不改动**：
- Node.js web 包，除了 worker 相关代码

**部署**：
- `cargo build --release --bin worker` → 单个 binary
- 系统启动时先启动 Rust worker，再启动 Node.js web
- Rust worker 连同一个 DATABASE_URL
- 通过 env var `WORKER_MODE=true` 禁用 Node.js worker

**回滚**：
- 停止 Rust worker，重新启动 Node.js worker (`pnpm run dev:worker`)
- pg_jobs 表格式不变，Node.js worker 可立即恢复消费

**验证清单**：
- [ ] Rust worker 消费 pg_jobs 的 ingestion-queue，写入 traces/observations/scores
- [ ] Rust worker 处理 webhook-queue，发送 HTTP webhook
- [ ] Rust worker 处理 notification-queue，发送邮件
- [ ] Rust worker 启动时执行种子脚本（evaluators, dashboards, model prices）
- [ ] 内存 < 50MB（release build）
- [ ] 与 Node.js web 共存 24h 无冲突
- [ ] 吞吐量不低于 Node.js worker

### Phase 2: API 迁移（预计 4-6 周）

**目标**：Rust API server 替代 Node.js 的 tRPC + REST API，Next.js 纯前端。

**改动范围**：
- 实现 `langfuse-auth` (JWT, API key, RBAC)
- 实现 `langfuse-api` (axum app, routes, middleware, OpenAPI)
- 实现 `bin/server.rs` 入口
- 迁移各资源路由（按优先级）

**优先级顺序**：
1. `POST /api/public/ingestion` (SDK 事件入口)
2. `GET /api/public/health` (健康检查)
3. `GET /api/traces/*`, `GET /api/observations/*`, `GET /api/scores/*` (核心读)
4. `POST /api/auth/*` (登录/注册)
5. `GET /api/sessions/*`, `GET /api/datasets/*`, `GET /api/prompts/*`
6. 其余路由（dashboards, comments, media, evals, models, etc.）
7. 公开 REST API v2/v3

**Next.js 改动**：
- 移除 `web/src/server/` (36 files)
- 移除 `web/src/features/*/server/` (126 files)
- 移除 `web/src/pages/api/` 所有路由
- 新增 `web/src/lib/api-client.ts` (OpenAPI 生成的类型安全客户端)
- 修改所有页面中的 tRPC 调用 → `apiClient.GET/POST`
- 登录页面改为调 Rust `/api/auth/login` 而不是 NextAuth

**Next.js 保留**：
- `pages/` — 页面路由和 SSR
- `components/` — UI 组件
- `features/*/` — 仅保留前端部分（hooks, components, utils）
- `styles/` — Tailwind 样式

**APIGateway** (过渡期)：
在 Phase 2 初期，可以用 Next.js 的 `next.config.mjs` 配置 proxy：
```js
// web/next.config.mjs — 过渡期
async rewrites() {
  return [
    { source: '/api/:path*', destination: 'http://localhost:8080/api/:path*' }
  ]
}
```
这样前端代码可以平滑过渡：先改 proxy 指向 Rust，验证通过后再移除 tRPC 代码。

**验证清单**：
- [ ] SDK POST /api/public/ingestion 正常写入数据
- [ ] 前端页面 SSR 渲染正常（trace 列表、session 详情、dashboard）
- [ ] 所有 CRUD 操作正常（创建/编辑/删除 trace, score, prompt, dataset 等）
- [ ] 登录/注册流程正常
- [ ] RBAC 权限检查正常（不同角色的用户看到不同内容）
- [ ] API Key 验证正常（Basic + Bearer）
- [ ] OpenAPI 文档可访问 (/api/docs)
- [ ] 前端 TypeScript 类型与 Rust API 类型一致
- [ ] 内存：Rust server < 100MB + Next.js < 200MB = 总计 < 300MB

### Phase 3: 收口（预计 1-2 周）

**目标**：清理 Node.js 残留，统一构建流程。

**改动**：
- 移除 `worker/` 目录
- 移除 `packages/shared/src/server/` 中已迁移到 Rust 的代码
- 移除 `web/` 中 server/ 和 api/ 目录
- 移除 BullMQ、ioredis、NextAuth 等依赖
- 清理 `package.json` 和 `.env` 中的废弃变量
- 更新 `README.md`、`AGENTS.md` 文档
- 更新 CI/CD pipeline

**验证清单**：
- [ ] `pnpm run lint && pnpm run typecheck` 通过（前端代码）
- [ ] `cargo clippy && cargo test` 通过（Rust 代码）
- [ ] 端到端测试：SDK 事件 → 数据库 → 前端展示
- [ ] 所有种子场景可执行

## 10. 验证策略

### 10.1 单元测试

```rust
// crates/langfuse-ingestion/tests/merger_test.rs
#[tokio::test]
async fn test_merge_trace_events_preserves_immutable_keys() {
    let events = vec![
        create_event("id1", "2024-01-01", Some("input1"), None),
        create_event("id2", "2024-01-02", None, Some("output2")),
    ];
    let result = merge_trace_records(&events, None);
    assert_eq!(result.id, "id1");            // 第一个事件的 id 不可变
    assert_eq!(result.input, Some("input1"));
    assert_eq!(result.output, Some("output2"));
}

#[tokio::test]
async fn test_batch_writer_insert_on_conflict() {
    // 测试 PgWriter 批量写入 + 冲突更新
}
```

### 10.2 集成测试

```rust
// bin/tests/integration/
// 启动真实 PostgreSQL (testcontainers)
// 1. 插入 job 到 pg_jobs
// 2. Rust worker 消费
// 3. 检查 traces/observations/scores 写入正确
```

### 10.3 对比测试 (Phase 1)

```bash
# 启动两个 worker 同时消费（不同 worker_id）
# 对比输出数据的行数、字段值
pgbench -f compare.sql -t 1000
```

### 10.4 前端回归测试

```bash
# Phase 2: 回归测试
pnpm --filter web run test           # server tests (迁移前)
pnpm --filter web run test-client    # client tests
pnpm --filter web run test:e2e       # E2E tests
```

## 11. 内存与性能估算

| 组件 | Node.js (当前) | Rust (目标) | 减少比例 |
|------|---------------|-------------|---------|
| API Server | Next.js dev 800-1200MB | axum release 30-50MB | 95%+ |
| Queue Consumer | tsx worker 50-80MB | tokio release 20-30MB | 60%+ |
| Next.js 纯前端 | N/A (当前混合) | next start 100-150MB | — |
| **总计** | **850-1280MB** | **150-230MB** | **75-82%** |

**注意事项**：
- 冷启动：Rust release binary < 2 秒 (vs Node.js worker 3-5 秒)
- 吞吐量：tokio 多线程 + sqlx 连接池 vs Node.js 单线程事件循环，理论提升 2-5x
- Prisma 连接池 (46 连接 by default) → sqlx 连接池 (20 连接)，连接数减少一半

## 12. 风险分析

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|---------|
| 复杂 SQL 翻译出错 | 中 | 高 | Phase 1 对比测试：新旧 worker 双跑，对比写入数据 |
| JWT session 与 NextAuth 不兼容 | 中 | 中 | Phase 2 先做 proxy 过渡，前后端用同一 JWT secret |
| 性能退步（某些查询） | 低 | 高 | sqlx query_as! 编译期校验 + EXPLAIN ANALYZE 验证 |
| 类型安全断裂 | 低 | 中 | OpenAPI + openapi-typescript 恢复编译期检查 |
| 迁移周期过长 | 中 | 中 | Phase 1 先验证 Worker 迁移可行性，再决定是否继续 Phase 2 |
| 团队 Rust 经验不足 | 高 | 中 | Phase 1 本身就是学习曲线最低的部分 (无 API, 无 Auth) |

## 13. LexQA 对接带来的变更（OTLP / 直写 / 计价 / 容器化）

本节记录为满足同级项目 **LexQA** 的集成要求所做的扩展。全文见
[`lexqa-integration.md`](./lexqa-integration.md)。

### 13.1 需求

LexQA（Go）内置 Langfuse 可观测集成，通过 **OTLP/HTTP 直写**上报五类模型调用：

```text
POST /api/public/otel/v1/traces
Authorization: Basic base64(public_key:secret_key)
x-langfuse-ingestion-version: 4
Content-Type: application/x-protobuf        (otlptracehttp 默认，且默认 gzip)
```

其约定（见 `LexQA/internal/tracing/langfuse/{events,tracer}.go`）：根 span 带
`langfuse.observation.type=trace` 与 `langfuse.trace.*`；子 span 带
`langfuse.observation.*`；用量放在 `langfuse.observation.usage_details`（JSON 字符串，
键为 `input`/`output`/`total`/`cache_read_input_tokens`/`cache_creation_input_tokens`/
`cache_miss_input_tokens`）；用户与会话用 `user.id` / `session.id`。

### 13.2 新增端点

| 端点 | 编码 | 说明 |
|---|---|---|
| `POST /api/public/otel/v1/traces` | protobuf / JSON，支持 `Content-Encoding: gzip` | 同步直写，部分成功用 OTLP `partialSuccess` 承载 |
| `POST /api/public/ingestion` | JSON | 改为**同步**处理（原为入队 + 5s 延迟） |
| `GET /api/public/traces`、`/traces/{id}` | — | 分页 + 官方过滤集；详情内嵌 observations/scores |
| `GET /api/public/sessions`、`/sessions/{id}` | — | |
| `GET /api/public/metrics/daily` | — | 按天 + 按模型/单位的用量与费用 |
| `GET`/`POST /api/public/models`、`GET`/`DELETE /models/{id}` | — | 含分项单价 `pricingTiers` |
| `GET /api/public/v2/{metrics,observations}` | — | 返回 **501**，见 13.6 |

`x-langfuse-ingestion-version` 被接受但**不校验**：上游用它在两条写入路径间分流，
本分支只有一条。

### 13.3 写入路径统一

原设计是 SDK 事件 → `pg_jobs` 队列 → `BatchWriter`（500ms 定时 flush / 1000 条早刷）。
两条问题：

1. 可见性依赖无关流量：LexQA 文档承诺「等 3 秒后在 Traces 页可见」，而队列延迟硬编码 5s。
2. 事件 → 列的映射在 `service.rs` 与 OTLP 路径各写一份，且已经漂移
   （`ingestion.rs` 内联的 match 与 `validator::get_entity_type` 语义不一致）。

现统一为 `langfuse-ingestion/src/sink.rs` 单一同步写入点，`BatchWriter` 已删除。
`langfuse-ingestion/src/otel.rs` 负责 OTLP 属性 → 记录的映射，复用同一 sink。

### 13.4 数据模型扩展

`observations` 新增两列（迁移 `20260918000000_add_observation_usage_and_cost_details`）：

| 列 | 类型 | 用途 |
|---|---|---|
| `usage_details` | jsonb | 按 usage type 的用量明细 |
| `cost_details` | jsonb | 按 usage type 的费用明细 |

原有 `prompt_tokens`/`completion_tokens`/`total_tokens` 保留为同一份数据的派生汇总。
不新增价格列 —— 已有的 `models` + `prices`（`usage_type` 唯一约束）足以表达分项单价。

### 13.5 计价

`langfuse-ingestion/src/pricing.rs`，在 `sink::write_observation` 中调用。复刻官方
`IngestionService.calculateUsageCosts`（随 ClickHouse 一起删掉的那份 TS 实现）：

```text
cost_details[k]    = price[usage_type = k] × usage_details[k]   对每个有单价的 k
cost_details.total = cost_details.total ?? Σ cost_details

calculated_input_cost  = Σ cost_details[k] where k.startsWith("input")
calculated_output_cost = Σ cost_details[k] where k.startsWith("output")
calculated_total_cost  = cost_details.total
```

几处必须记住的约束：

- **`cost_details` 以 usage type 为键**，不是固定 input/output 两档 —— 这是 cache 读写能
  各自成行、各自计费的前提，也是 UI「Cost breakdown」逐项展开的依据。
- **值是 JSON number 且必须带 `total`**。前端 `CostDetails` schema 只保留
  `typeof value === "number"` 的条目；表格总费用列读 `cost_details['total']`。写成字符串
  会被静默丢弃（本实现最初就是字符串，是缺陷）。
- **只选中一个 pricing tier**：非 default 档按 `priority` 升序取第一个条件全满足的，否则
  default 档；非 default 档的空条件视为不匹配（官方 `evaluateConditions` 对空数组返回
  false，Rust 的 `all()` 对空迭代器返回 true —— 两者必须显式对齐）。按 model 扫全部
  `prices` 会把互斥分档混在一起。
- **客户端已给 `langfuse.observation.cost_details` 时不计算**，原样规整后使用。
- 单价单位 = **每 token**（与预置价一致：gpt-4o `input = 0.0000025`）。按
  `models.match_pattern`（正则）匹配，项目自定义优先于内置，同作用域取最新 `start_date`。
- 非 token 单位（`SECONDS` 等）：档位无任何匹配单价时退回 `models.total_price × 用量`，
  只为没迁到 `prices` 表的旧模型保留。此时 `calculated_input_cost` / `_output_cost` 为
  NULL、只有 `calculated_total_cost` 有值。
- 同时回填 `internal_model` / `internal_model_id`，便于后续按模型重算。

两个与精度有关的坑：

- `Decimal::to_f64()` 把最大 96 bit 的 mantissa 除以 `10^scale`，而 `numeric(65,30)` 返回的
  值带 30 位小数，mantissa 超出 f64 的 53 bit 精确整数范围 → `1000` 会变成
  `1000.0000000000001`。统一走 `langfuse-core::money::to_json_f64`（先 `normalize()`
  去掉尾随零再转换）。
- 计价全程保持在 `Decimal` 上，只在写出 `cost_details` 时降为 f64；先转 f64 再相乘会把
  `0.0024` 存成 `0.002400000000000000200000000000`。

### 13.6 读路径与 UI 适配

外部 `Observation` 形状里 `inputCost` / `outputCost` / `totalCost` **不是那三个列**，而是
从 `cost_details` 按上面的前缀规则归约出来的（官方 `observations_converters.ts` 同样如此）。
`ObservationResponse` 因此同时给：

- `usageDetails` / `costDetails`（原始 jsonb）
- `inputUsage` / `outputUsage` / `totalUsage`（由 `usage_details` 归约）
- `inputCost` / `outputCost` / `totalCost` 与 `calculatedInputCost` / … （同一份值，两个
  拼写）

配套补齐的控制台依赖端点（都不在 `/api/public` 下）：

| 端点 | 用途 |
|---|---|
| `GET /api/traces/{traceId}/full` | trace 详情页 span 树、分项 usage / 费用 |
| `GET /api/observations?trace_id=&type=&cursor=` | `type` 与 `cursor` 此前被接受但**被忽略**，即 `?type=GENERATION` 返回全部、翻页不前进；现已在 SQL 层生效 |
| `GET /api/sessions/hasAny`、`/hasAnyFromEvents` | Sessions 页在空态与表格间二选一（缺失则永远停在空态） |
| `GET /api/sessions/metrics`、`/metricsFromEvents` | Sessions 表的时长 / trace 数 / 费用 / token 列 |
| `GET /api/models` | Settings → Model Definitions 列表，含 `pricingTiers`（`prices` 为裸 number，非 `{price}`）与 `totalCount` |

前端三处相应修正：

- `web/src/utils/api.ts`：`sessions.hasAny` / `sessions.metrics` 此前落到 placeholder；
  补上真实映射，并对 metrics 行的三个费用字段还原 `Decimal`（旧 tRPC 靠 superjson 保类
  实例，JSON 只能传 number，而表格调 `.toNumber()`）。`autoParseDates` 不再改写
  `usage_details` / `cost_details` / `prices` 的键——那些键是数据不是字段名。
- `web/src/hooks/useAuth.ts`：`fetchSession` 原为**不发请求**的硬编码常量，其
  `user.organizations` 只含两个固定组织；任何不在该列表内的项目（如 bootstrap 建的
  LexQA）`useQueryProject` 都解析为 null，于是整个 Settings 分区渲染空白。改为真实请求
  `/api/auth/session`（由 `next.config.mjs` rewrite 到 Rust）。
  该常量**已彻底删除**：取不到会话就是未登录，交给 `useAuthGuard` 跳
  `/auth/sign-in`。留一个「API 不通时假装已登录」的分支，等于让无凭证的访问者看到一个
  连着真实数据的控制台（见 §13.8）。
- `web/src/utils/api.ts`：`LIST_ALIASES` 增加 `getAll`。

### 13.7 已知未实现

`GET /api/public/v2/metrics` 与 `GET /api/public/v2/observations` 返回 501。

- v2 metrics 是通用 OLAP 查询引擎（3 view、约 20 维度、约 15 度量、11 种聚合含
  p50–p99 与直方图、过滤算子矩阵、时间粒度、排序），响应契约为开放的
  `list<map<string, unknown>>`。部分实现会给出「看似合理但错误」的数字。
  按天/按模型的用量与费用用 `metrics/daily`。
- v2 observations 是游标分页 + 10 个字段组 + 元数据截断。observations 目前经
  `GET /api/public/traces/{traceId}` 获取。

### 13.8 内部 API 的鉴权（已修复）

修复之前，`middleware/auth.rs::verify_session` 是硬编码 no-op：对每个请求注入同一个
`Session`，**不校验任何凭证**。任何能访问端口的客户端都可读写全部 traces、列项目、创建
API key：

```console
$ curl -s -o /dev/null -w '%{http_code}\n' http://<host>:8010/api/projects
200                      # 修复前：无任何凭证
```

同一时期 `routes/traces.rs::verify_project_access` 是 `fn(...) -> Ok(())`，`api_keys` 的
四个 handler 完全不校验 project 归属，`web` 侧的 `signIn`/`signOut` 是立即重新登录的空
函数。即"认证"与"授权"两层同时缺失。

现在：

| 层 | 实现 | 证据 |
|---|---|---|
| 登录 | `POST /api/auth/login` 校验 `users.password`（bcrypt），签发 `JWT_SECRET` 签名的 JWT | `routes/auth.rs` |
| 会话 | `HttpOnly` cookie `langfuse_session`；`verify_session` 校验签名与 `exp`，注入 `Session` | `middleware/auth.rs` |
| 授权 | 每个带 `project_id` 的 handler 调 `require_project_access`，按 `organization_memberships` 判定，不属于返回 **404** | `langfuse-auth/src/rbac.rs` |
| 抗枚举 | 未知邮箱与错误口令走同一条 bcrypt 耗时路径、返回同一个响应体 | `password.rs::equalize_timing` |
| 抗爆破 | 按**账号**（非 IP）计数，`MAX_FAILURES` 次失败锁 `WINDOW` | `throttle.rs` |
| 登出 | `POST /api/auth/logout` 下发 `Max-Age=0` 的同名同属性 cookie | `session_cookie.rs` |
| 跨域 | 默认不允许；`LANGFUSE_CORS_ALLOWED_ORIGINS` 显式列出 | `app.rs::cors_layer` |

按 IP 限流在这里是错的：浏览器经 Next.js rewrite 到达 API，所有请求同源，IP 计数要么
永不触发，要么一次攻击锁死全部用户。

回归测试：`crates/langfuse-api/tests/private_api_auth.rs`，含"API Key 不是会话、会话不是
API Key"的交叉验证。

### 13.9 容器化

单镜像（`Dockerfile`）跑两个进程，不再需要上游的 ClickHouse / MinIO / Redis / 独立 worker：

```text
node  :3000  Next.js standalone（UI + rewrite /api/public/* → :8010）
rust  :8010  axum API + 5 个队列消费者（bin/server.rs 已 tokio::spawn 内联）
```

`docker/entrypoint.sh` 用 `tini` 作 PID 1 并监督两个子进程，任一退出即整体退出。
`bin/worker.rs` **不进镜像**：与 server 内联的消费者重复，同跑会让每个队列出现
两批工作进程争抢同一批 `pg_jobs` 行。

新增队列注册的单一入口 `langfuse-rs/src/queues.rs::register_all_queues`，并发数从
`langfuse_core::Config` 读取（此前 `config.rs` 的 `Config::from_env` 零调用，
并发数是两处重复的硬编码字面量）。

> 监听地址改用 `LANGFUSE_BIND_ADDRESS`（默认 `0.0.0.0`）。原先读 `HOSTNAME`，
> 而 Docker 会把 `HOSTNAME` 设为容器 ID，容器内会因此绑定失败。
