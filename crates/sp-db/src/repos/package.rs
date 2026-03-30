use sqlx::PgPool;
use uuid::Uuid;

use sp_core::Ecosystem;

use crate::models::PackageRow;

pub struct PackageRepo;

impl PackageRepo {
    pub async fn find_by_name(
        pool: &PgPool,
        ecosystem: Ecosystem,
        normalized_name: &str,
    ) -> Result<Option<PackageRow>, sqlx::Error> {
        sqlx::query_as::<_, PackageRow>(
            "SELECT * FROM packages WHERE ecosystem = $1 AND normalized_name = $2",
        )
        .bind(ecosystem)
        .bind(normalized_name)
        .fetch_optional(pool)
        .await
    }

    pub async fn find_or_create(
        pool: &PgPool,
        ecosystem: Ecosystem,
        name: &str,
        normalized_name: &str,
    ) -> Result<PackageRow, sqlx::Error> {
        // Try insert, on conflict return existing.
        sqlx::query_as::<_, PackageRow>(
            r#"
            INSERT INTO packages (ecosystem, name, normalized_name)
            VALUES ($1, $2, $3)
            ON CONFLICT (ecosystem, normalized_name) DO UPDATE SET updated_at = now()
            RETURNING *
            "#,
        )
        .bind(ecosystem)
        .bind(name)
        .bind(normalized_name)
        .fetch_one(pool)
        .await
    }

    pub async fn list_all(
        pool: &PgPool,
        ecosystem: Ecosystem,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PackageRow>, sqlx::Error> {
        sqlx::query_as::<_, PackageRow>(
            "SELECT * FROM packages WHERE ecosystem = $1 ORDER BY normalized_name LIMIT $2 OFFSET $3",
        )
        .bind(ecosystem)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    }

    pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<PackageRow>, sqlx::Error> {
        sqlx::query_as::<_, PackageRow>("SELECT * FROM packages WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
    }
}
