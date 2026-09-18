//! 统一 API 响应格式
use serde::Serialize;
use utoipa::ToSchema;

/// 通用列表响应 { data: { items: [...] }, meta: {...}, error: null }
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiResponse<T: Serialize> {
    pub data: Option<T>,
    pub meta: Option<PaginationMeta>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PaginationMeta {
    pub cursor: Option<String>,
    pub has_more: bool,
    pub total: Option<i64>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self { data: Some(data), meta: None, error: None }
    }

    pub fn with_meta(data: T, cursor: Option<String>, has_more: bool, total: Option<i64>) -> Self {
        Self {
            data: Some(data),
            meta: Some(PaginationMeta { cursor, has_more, total }),
            error: None,
        }
    }
}

/// Error body for the public API (`/api/public/*`), matching the official
/// Langfuse shape `{ "message": ..., "error": ... }`.
///
/// The public API is consumed by machine clients — Langfuse SDKs, OTLP
/// exporters, LexQA — which surface `message` in their error paths. A bare
/// status code leaves a 401 indistinguishable from a crashed upstream, so
/// every public failure goes through this helper.
pub fn public_error(
    status: axum::http::StatusCode,
    message: impl Into<String>,
    error: impl Into<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        status,
        axum::Json(serde_json::json!({
            "message": message.into(),
            "error": error.into(),
        })),
    )
        .into_response()
}

// =====================================================================
// 具体资源响应数据 — 用于 OpenAPI 类型推断
// =====================================================================

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceListData {
    pub traces: Vec<super::routes::traces::TraceResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ObservationListData {
    pub observations: Vec<super::routes::observations::ObservationResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScoreListData {
    pub scores: Vec<super::routes::scores::ScoreResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionListData {
    pub sessions: Vec<super::routes::sessions::SessionResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectListData {
    pub projects: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrganizationListData {
    pub organizations: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DatasetListData {
    pub datasets: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PromptListData {
    pub prompts: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ModelListData {
    pub models: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardListData {
    pub dashboards: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommentListData {
    pub comments: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MediaListData {
    pub media: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EvalTemplateListData {
    pub eval_templates: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserListData {
    pub users: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AutomationListData {
    pub automations: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MonitorListData {
    pub monitors: Vec<serde_json::Value>,
}

// Auth
#[derive(Debug, Serialize, ToSchema)]
pub struct LoginResponseData {
    pub token: String,
    pub user: super::routes::auth::UserInfo,
}
