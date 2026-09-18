use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
// use utoipa::ToSchema; // Phase 2

use crate::enums::{
    DatasetStatus, JobConfigState, JobExecutionStatus, JobType, ObservationLevel,
    ObservationType, Role, ScoreDataType, ScoreSource,
};

// ============================================================================
// 租户 & 组织
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub cloud_config: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct Project {
    pub id: String,
    pub org_id: String,
    pub name: String,
    pub retention_days: Option<i32>,
    pub has_traces: Option<bool>,
    pub public: bool,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct User {
    pub id: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub email_verified: Option<DateTime<Utc>>,
    pub image: Option<String>,
    pub admin: bool,
    pub feature_flags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// 成员
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct OrganizationMembership {
    pub id: String,
    pub user_id: String,
    pub org_id: String,
    pub role: Role,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct ProjectMembership {
    pub project_id: String,
    pub user_id: String,
    pub role: Role,
    pub org_membership_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// API Keys
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct ApiKey {
    pub id: String,
    pub public_key: String,
    pub hashed_secret_key: String,
    pub fast_hashed_secret_key: Option<String>,
    pub display_secret_key: Option<String>,
    pub scope: String,
    pub project_id: Option<String>,
    pub org_id: Option<String>,
    pub is_in_app_agent_key: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// 遥测数据 - Traces
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct Trace {
    pub id: String,
    pub project_id: String,
    pub external_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub name: Option<String>,
    pub user_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub release: Option<String>,
    pub version: Option<String>,
    pub public: bool,
    pub bookmarked: bool,
    pub tags: Vec<String>,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub session_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 用于 INSERT/UPDATE 的 trace 记录 (不含 read-only 字段)
/// 注: traces 表无 environment 列
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRecord {
    pub id: String,
    pub project_id: String,
    pub external_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub name: Option<String>,
    pub user_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub release: Option<String>,
    pub version: Option<String>,
    pub public: Option<bool>,
    pub bookmarked: Option<bool>,
    pub tags: Option<Vec<String>>,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub session_id: Option<String>,
}

// ============================================================================
// 遥测数据 - Observations
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct Observation {
    pub id: String,
    pub trace_id: Option<String>,
    pub project_id: String,
    /// Langfuse payloads key this as `type`; the Rust field cannot be named
    /// `type`, so the mapping has to be explicit.
    #[serde(rename = "type")]
    pub obs_type: ObservationType,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub name: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub parent_observation_id: Option<String>,
    pub level: ObservationLevel,
    pub status_message: Option<String>,
    pub version: Option<String>,
    /// 模型名 (用户看到的)
    pub model: Option<String>,
    /// 内部模型名
    pub internal_model: Option<String>,
    pub internal_model_id: Option<String>,
    pub model_parameters: Option<serde_json::Value>,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
    pub unit: Option<String>,
    /// 用量明细，按 usage type 索引 (input / output / total /
    /// cache_read_input_tokens / cache_creation_input_tokens /
    /// cache_miss_input_tokens)。prompt_tokens / completion_tokens /
    /// total_tokens 是同一份数据的派生汇总。
    pub usage_details: Option<serde_json::Value>,
    pub input_cost: Option<Decimal>,
    pub output_cost: Option<Decimal>,
    pub total_cost: Option<Decimal>,
    /// 费用明细，键与 usage_details 一致
    pub cost_details: Option<serde_json::Value>,
    pub calculated_input_cost: Option<Decimal>,
    pub calculated_output_cost: Option<Decimal>,
    pub calculated_total_cost: Option<Decimal>,
    pub completion_start_time: Option<DateTime<Utc>>,
    pub prompt_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 注: observations 表无 environment, prompt_name, prompt_version 列
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationRecord {
    pub id: String,
    pub trace_id: Option<String>,
    pub project_id: String,
    /// See [`Observation::obs_type`] — the wire key is `type`.
    #[serde(rename = "type")]
    pub obs_type: ObservationType,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub name: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub parent_observation_id: Option<String>,
    pub level: Option<ObservationLevel>,
    pub status_message: Option<String>,
    pub version: Option<String>,
    pub model: Option<String>,
    pub internal_model: Option<String>,
    pub internal_model_id: Option<String>,
    /// Langfuse SDKs send this key as camelCase (`modelParameters`), which the
    /// merger's key normalisation deliberately leaves alone — so the alias is
    /// what keeps model parameters from being silently dropped on ingest.
    #[serde(alias = "modelParameters")]
    pub model_parameters: Option<serde_json::Value>,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
    pub unit: Option<String>,
    /// 用量明细，按 usage type 索引；见 `Observation::usage_details`
    pub usage_details: Option<serde_json::Value>,
    pub input_cost: Option<Decimal>,
    pub output_cost: Option<Decimal>,
    pub total_cost: Option<Decimal>,
    /// 费用明细，键与 usage_details 一致
    pub cost_details: Option<serde_json::Value>,
    pub calculated_input_cost: Option<Decimal>,
    pub calculated_output_cost: Option<Decimal>,
    pub calculated_total_cost: Option<Decimal>,
    pub completion_start_time: Option<DateTime<Utc>>,
    pub prompt_id: Option<String>,
}

// ============================================================================
// 遥测数据 - Scores
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct Score {
    pub id: String,
    pub project_id: String,
    pub timestamp: DateTime<Utc>,
    pub name: String,
    pub value: Option<f64>,
    pub source: ScoreSource,
    pub author_user_id: Option<String>,
    pub comment: Option<String>,
    pub trace_id: Option<String>,
    pub observation_id: Option<String>,
    pub config_id: Option<String>,
    pub string_value: Option<String>,
    pub queue_id: Option<String>,
    pub data_type: ScoreDataType,
    pub metadata: Option<serde_json::Value>,
    pub execution_trace_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreRecord {
    pub id: String,
    pub project_id: String,
    pub timestamp: DateTime<Utc>,
    pub name: String,
    pub value: Option<f64>,
    pub source: ScoreSource,
    pub author_user_id: Option<String>,
    pub comment: Option<String>,
    pub trace_id: Option<String>,
    pub observation_id: Option<String>,
    pub config_id: Option<String>,
    pub string_value: Option<String>,
    pub queue_id: Option<String>,
    pub data_type: Option<ScoreDataType>,
    pub metadata: Option<serde_json::Value>,
    pub execution_trace_id: Option<String>,
}

// ============================================================================
// Score Config
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct ScoreConfig {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub data_type: ScoreDataType,
    pub is_archived: bool,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub categories: Option<serde_json::Value>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Sessions
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct TraceSession {
    pub id: String,
    pub project_id: String,
    pub bookmarked: bool,
    pub public: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub environment: Option<String>,
}

// ============================================================================
// Datasets
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct Dataset {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub input_schema: Option<serde_json::Value>,
    pub expected_output_schema: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct DatasetItem {
    pub id: String,
    pub project_id: String,
    pub dataset_id: String,
    pub status: DatasetStatus,
    pub input: Option<serde_json::Value>,
    pub expected_output: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub source_trace_id: Option<String>,
    pub source_observation_id: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct DatasetRun {
    pub id: String,
    pub project_id: String,
    pub dataset_id: String,
    pub name: String,
    pub description: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct DatasetRunItem {
    pub id: String,
    pub project_id: String,
    pub dataset_run_id: String,
    pub dataset_item_id: String,
    pub trace_id: Option<String>,
    pub observation_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Prompts
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct Prompt {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version: i32,
    pub prompt: serde_json::Value,
    pub r#type: Option<String>,
    pub config: Option<serde_json::Value>,
    pub tags: Vec<String>,
    pub labels: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct Model {
    pub id: String,
    pub project_id: String,
    pub model_name: String,
    pub match_pattern: String,
    pub start_date: Option<DateTime<Utc>>,
    pub input_price: Option<Decimal>,
    pub output_price: Option<Decimal>,
    pub total_price: Option<Decimal>,
    pub unit: String,
    pub tokenizer_id: Option<String>,
    pub tokenizer_config: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Evals & Jobs
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct EvalTemplate {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version: i32,
    pub prompt: serde_json::Value,
    pub r#type: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub model_params: Option<serde_json::Value>,
    pub vars: Vec<String>,
    pub output_schema: Option<serde_json::Value>,
    pub source_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct JobConfiguration {
    pub id: String,
    pub project_id: String,
    pub job_type: JobType,
    pub status: JobConfigState,
    pub blocked_at: Option<DateTime<Utc>>,
    pub block_reason: Option<String>,
    pub eval_template_id: Option<String>,
    pub score_name: Option<String>,
    pub filter: serde_json::Value,
    pub target_object: Option<String>,
    pub variable_mapping: Option<serde_json::Value>,
    pub sampling: Option<f64>,
    pub delay: Option<i32>,
    pub time_scope: Option<Vec<String>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct JobExecution {
    pub id: String,
    pub project_id: String,
    pub job_configuration_id: String,
    pub status: JobExecutionStatus,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub job_input_trace_id: Option<String>,
    pub job_input_observation_id: Option<String>,
    pub job_input_dataset_item_id: Option<String>,
    pub job_output_score_id: Option<String>,
    pub execution_trace_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// 队列 & 事件
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PgJob {
    pub id: i64,
    pub job_id: String,
    pub queue_name: String,
    pub payload: serde_json::Value,
    pub state: String,
    pub run_at: DateTime<Utc>,
    pub max_attempts: i32,
    pub attempts: i32,
    pub backoff_type: Option<String>,
    pub backoff_delay: Option<i32>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub locked_by: Option<String>,
    pub locked_until: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionJobPayload {
    pub data: IngestionJobData,
    pub auth_check: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionJobData {
    pub r#type: String,
    pub event_body_id: String,
    pub events: Vec<serde_json::Value>,
    pub forward_to_events_table: Option<bool>,
}

// ============================================================================
// Comment & Notification
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct Comment {
    pub id: String,
    pub project_id: String,
    pub object_type: String,
    pub object_id: String,
    pub content: String,
    pub author_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct NotificationPreference {
    pub id: String,
    pub user_id: String,
    pub project_id: String,
    pub channel: String,
    pub notification_type: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Dashboard
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize/*, FromRow*/)]
pub struct Dashboard {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: Option<String>,
    pub definition: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
