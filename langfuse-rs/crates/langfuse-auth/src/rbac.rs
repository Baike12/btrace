//! 权限检查 — 项目级访问控制
//!
//! 控制台的所有私有路由都带一个 `project_id`，它**来自请求**而不是来自会话，
//! 因此每个 handler 都必须确认「当前会话用户确实属于拥有该项目的组织」。
//! 少了这一步，任何登录用户都能通过改一个查询参数读到别人的项目。

use sqlx::PgPool;

use langfuse_core::Result;

/// 判断用户能否访问某个项目。
///
/// 判定依据是**组织成员关系**：项目归属于某个组织，用户属于该组织即视为可访问。
/// 这与 `/api/auth/session` 告诉 UI 的可见项目集合一致——UI 用同一份 orgs →
/// projects 列表渲染项目选择器，两边口径不同会让「侧边栏看得见、点进去 403」。
///
/// `project_memberships` 只细化了用户在该项目内的角色（OWNER/MEMBER/VIEWER），
/// 不改变项目可见性，因此这里不参与判定。
pub async fn user_can_access_project(
    pool: &PgPool,
    user_id: &str,
    project_id: &str,
) -> Result<bool> {
    let allowed: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1
               FROM projects p
               JOIN organization_memberships om ON om.org_id = p.org_id
               WHERE p.id = $1 AND om.user_id = $2
           )"#,
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    Ok(allowed)
}

/// 判断用户能否访问某个组织。
pub async fn user_can_access_organization(
    pool: &PgPool,
    user_id: &str,
    org_id: &str,
) -> Result<bool> {
    let allowed: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1 FROM organization_memberships
               WHERE org_id = $1 AND user_id = $2
           )"#,
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    Ok(allowed)
}
