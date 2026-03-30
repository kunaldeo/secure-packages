use sqlx::PgPool;
use uuid::Uuid;

use crate::models::AuditLogRow;

pub struct AuditRepo;

impl AuditRepo {
    pub async fn create(
        pool: &PgPool,
        actor: &str,
        action: &str,
        target_type: Option<&str>,
        target_id: Option<Uuid>,
        details: Option<serde_json::Value>,
    ) -> Result<AuditLogRow, sqlx::Error> {
        sqlx::query_as::<_, AuditLogRow>(
            r#"
            INSERT INTO audit_log (actor, action, target_type, target_id, details_json)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(actor)
        .bind(action)
        .bind(target_type)
        .bind(target_id)
        .bind(details)
        .fetch_one(pool)
        .await
    }

    pub async fn list_recent(pool: &PgPool, limit: i64) -> Result<Vec<AuditLogRow>, sqlx::Error> {
        sqlx::query_as::<_, AuditLogRow>(
            "SELECT * FROM audit_log ORDER BY created_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}
