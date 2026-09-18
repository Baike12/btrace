use langfuse_core::Result;
use sqlx::{PgPool, Row};

/// Upsert dataset run item
pub async fn upsert_dataset_run_item(
    pool: &PgPool, id: &str, project_id: &str, dataset_run_id: &str,
    dataset_item_id: &str, trace_id: Option<&str>, observation_id: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO dataset_run_items (id, project_id, dataset_run_id, dataset_item_id, trace_id, observation_id)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (id, project_id) DO UPDATE SET
            trace_id = COALESCE(EXCLUDED.trace_id, dataset_run_items.trace_id),
            observation_id = COALESCE(EXCLUDED.observation_id, dataset_run_items.observation_id),
            updated_at = now()"#
    )
    .bind(id).bind(project_id).bind(dataset_run_id).bind(dataset_item_id)
    .bind(trace_id).bind(observation_id).execute(pool).await?;
    Ok(())
}

// Read queries
#[derive(Debug, Clone)]
pub struct DatasetRow {
    pub id: String, pub project_id: String, pub name: String,
    pub description: Option<String>, pub metadata: Option<serde_json::Value>,
    pub created_at: chrono::NaiveDateTime, pub updated_at: chrono::NaiveDateTime,
}

pub async fn list_datasets(pool: &PgPool, project_id: &str, limit: i64) -> Result<Vec<DatasetRow>> {
    let rows = sqlx::query(
        "SELECT id, project_id, name, description, metadata, created_at, updated_at
         FROM datasets WHERE project_id = $1 ORDER BY created_at DESC LIMIT $2"
    ).bind(project_id).bind(limit + 1).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r: sqlx::postgres::PgRow| DatasetRow {
        id: r.get("id"), project_id: r.get("project_id"), name: r.get("name"),
        description: r.get("description"), metadata: r.get("metadata"),
        created_at: r.get("created_at"), updated_at: r.get("updated_at"),
    }).collect())
}
