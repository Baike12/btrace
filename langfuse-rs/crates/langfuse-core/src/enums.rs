use serde::{Deserialize, Serialize};
// use sqlx::Type; // Phase 2 — needed for PG enum mapping when reading
// use utoipa::ToSchema; // Phase 2

/// 观测类型 - 对应 Prisma enum LegacyPrismaObservationType
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

#[serde(rename_all = "UPPERCASE")]
pub enum ObservationType {
    Span,
    Event,
    Generation,
    Agent,
    Tool,
    Chain,
    Retriever,
    Evaluator,
    Embedding,
    Guardrail,
}

impl std::fmt::Display for ObservationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ObservationType::Span => "SPAN",
            ObservationType::Event => "EVENT",
            ObservationType::Generation => "GENERATION",
            ObservationType::Agent => "AGENT",
            ObservationType::Tool => "TOOL",
            ObservationType::Chain => "CHAIN",
            ObservationType::Retriever => "RETRIEVER",
            ObservationType::Evaluator => "EVALUATOR",
            ObservationType::Embedding => "EMBEDDING",
            ObservationType::Guardrail => "GUARDRAIL",
        };
        write!(f, "{}", s)
    }
}

/// 观测日志级别
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

#[serde(rename_all = "UPPERCASE")]
pub enum ObservationLevel {
    Debug,
    Default,
    Warning,
    Error,
}

impl std::fmt::Display for ObservationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ObservationLevel::Debug => "DEBUG",
            ObservationLevel::Default => "DEFAULT",
            ObservationLevel::Warning => "WARNING",
            ObservationLevel::Error => "ERROR",
        };
        write!(f, "{}", s)
    }
}

/// Score 来源
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

#[serde(rename_all = "UPPERCASE")]
pub enum ScoreSource {
    Annotation,
    Api,
    Eval,
}

impl std::fmt::Display for ScoreSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ScoreSource::Annotation => "ANNOTATION",
            ScoreSource::Api => "API",
            ScoreSource::Eval => "EVAL",
        };
        write!(f, "{}", s)
    }
}

/// Score 数据类型
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

#[serde(rename_all = "UPPERCASE")]
pub enum ScoreDataType {
    Numeric,
    Categorical,
    Boolean,
    Correction,
    Text,
}

impl std::fmt::Display for ScoreDataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ScoreDataType::Numeric => "NUMERIC",
            ScoreDataType::Categorical => "CATEGORICAL",
            ScoreDataType::Boolean => "BOOLEAN",
            ScoreDataType::Correction => "CORRECTION",
            ScoreDataType::Text => "TEXT",
        };
        write!(f, "{}", s)
    }
}

/// 用户角色 - 对应 Prisma enum Role
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

#[serde(rename_all = "UPPERCASE")]
pub enum Role {
    Owner,
    Admin,
    Member,
    Viewer,
    None,
}

impl Role {
    /// 角色优先级数值 (用于权限比较)
    pub fn order(&self) -> i32 {
        match self {
            Role::Owner => 4,
            Role::Admin => 3,
            Role::Member => 2,
            Role::Viewer => 1,
            Role::None => 0,
        }
    }
}

/// API Key 作用域
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

#[serde(rename_all = "UPPERCASE")]
pub enum ApiKeyScope {
    Organization,
    Project,
}

/// 数据集状态
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

#[serde(rename_all = "UPPERCASE")]
pub enum DatasetStatus {
    Active,
    Archived,
}

/// 评论对象类型
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

#[serde(rename_all = "UPPERCASE")]
pub enum CommentObjectType {
    Trace,
    Observation,
    Session,
    Prompt,
}

/// Job 类型
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

pub enum JobType {
    Eval,
}

/// Job 配置状态
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

pub enum JobConfigState {
    Active,
    Inactive,
}

/// Job 执行状态
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

pub enum JobExecutionStatus {
    Pending,
    Completed,
    Failed,
    Cancelled,
}

/// 队列名称 - 对应原有的 QueueName enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QueueName {
    IngestionQueue,
    WebhookQueue,
    NotificationQueue,
    EntityChangeQueue,
    MonitorQueue,
    TraceUpsert,
}

impl QueueName {
    pub fn as_str(&self) -> &'static str {
        match self {
            QueueName::IngestionQueue => "ingestion-queue",
            QueueName::WebhookQueue => "webhook-queue",
            QueueName::NotificationQueue => "notification-queue",
            QueueName::EntityChangeQueue => "entity-change-queue",
            QueueName::MonitorQueue => "monitor-queue",
            QueueName::TraceUpsert => "trace-upsert",
        }
    }
}

/// 队列 Job 状态
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

pub enum JobState {
    Waiting,
    Active,
    Completed,
    Failed,
    Delayed,
}

/// Eval 模板类型
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

pub enum EvalTemplateType {
    
    LlmAsJudge,
    
    Code,
}

/// 通知类型
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

pub enum NotificationType {
    
    CommentMention,
}

/// 通知渠道
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]

#[serde(rename_all = "lowercase")]
pub enum NotificationChannel {
    Email,
}

/// 退避类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackoffType {
    Fixed,
    Exponential,
}

impl std::fmt::Display for BackoffType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackoffType::Fixed => write!(f, "fixed"),
            BackoffType::Exponential => write!(f, "exponential"),
        }
    }
}

/// 实体类型 (用于 ingestion 事件分组)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityType {
    Trace,
    Observation,
    Score,
    DatasetRunItem,
}

impl EntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityType::Trace => "trace",
            EntityType::Observation => "observation",
            EntityType::Score => "score",
            EntityType::DatasetRunItem => "dataset_run_item",
        }
    }
}

/// 表名 (用于 BatchWriter)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableName {
    Traces,
    Observations,
    Scores,
    DatasetRunItems,
    Events,
    TraceSessions,
}

impl TableName {
    pub fn as_str(&self) -> &'static str {
        match self {
            TableName::Traces => "traces",
            TableName::Observations => "observations",
            TableName::Scores => "scores",
            TableName::DatasetRunItems => "dataset_run_items",
            TableName::Events => "events",
            TableName::TraceSessions => "trace_sessions",
        }
    }
}
