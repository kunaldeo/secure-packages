use sqlx::PgPool;
use uuid::Uuid;

use sp_core::AnalysisStatus;

use crate::models::PackageVersionRow;

pub struct VersionRepo;

impl VersionRepo {
    pub async fn find(
        pool: &PgPool,
        package_id: Uuid,
        version: &str,
    ) -> Result<Option<PackageVersionRow>, sqlx::Error> {
        sqlx::query_as::<_, PackageVersionRow>(
            "SELECT * FROM package_versions WHERE package_id = $1 AND version = $2",
        )
        .bind(package_id)
        .bind(version)
        .fetch_optional(pool)
        .await
    }

    /// Idempotent: returns existing row if version already exists (any status),
    /// otherwise creates a new pending row. Prevents duplicate jobs on concurrent requests.
    pub async fn find_or_create_pending(
        pool: &PgPool,
        package_id: Uuid,
        version: &str,
    ) -> Result<PackageVersionRow, sqlx::Error> {
        sqlx::query_as::<_, PackageVersionRow>(
            r#"
            INSERT INTO package_versions (package_id, version, status)
            VALUES ($1, $2, 'pending')
            ON CONFLICT (package_id, version) DO UPDATE SET updated_at = now()
            RETURNING *
            "#,
        )
        .bind(package_id)
        .bind(version)
        .fetch_one(pool)
        .await
    }

    pub async fn list_by_package(
        pool: &PgPool,
        package_id: Uuid,
    ) -> Result<Vec<PackageVersionRow>, sqlx::Error> {
        sqlx::query_as::<_, PackageVersionRow>(
            "SELECT * FROM package_versions WHERE package_id = $1 ORDER BY created_at DESC",
        )
        .bind(package_id)
        .fetch_all(pool)
        .await
    }

    pub async fn list_approved(
        pool: &PgPool,
        package_id: Uuid,
    ) -> Result<Vec<PackageVersionRow>, sqlx::Error> {
        sqlx::query_as::<_, PackageVersionRow>(
            "SELECT * FROM package_versions WHERE package_id = $1 AND status = 'approved' ORDER BY created_at DESC",
        )
        .bind(package_id)
        .fetch_all(pool)
        .await
    }

    pub async fn update_status(
        pool: &PgPool,
        id: Uuid,
        status: AnalysisStatus,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE package_versions SET status = $1, updated_at = now() WHERE id = $2")
            .bind(status)
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn update_status_with_error(
        pool: &PgPool,
        id: Uuid,
        status: AnalysisStatus,
        error_message: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE package_versions SET status = $1, error_message = $2, updated_at = now() WHERE id = $3",
        )
        .bind(status)
        .bind(error_message)
        .bind(id)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn update_source_hash(
        pool: &PgPool,
        id: Uuid,
        sha256: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE package_versions SET source_sha256 = $1, updated_at = now() WHERE id = $2",
        )
        .bind(sha256)
        .bind(id)
        .execute(pool)
        .await?;
        Ok(())
    }
}
