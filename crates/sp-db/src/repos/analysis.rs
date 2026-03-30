use sqlx::PgPool;
use uuid::Uuid;

use crate::models::AnalysisRecordRow;

pub struct NewAnalysisRecord {
    pub package_version_id: Uuid,
    pub analysis_type: String,
    pub static_scan: Option<serde_json::Value>,
    pub llm_result: Option<serde_json::Value>,
    pub diff_summary: Option<serde_json::Value>,
    pub verdict: String,
    pub risk_score: Option<f32>,
    pub reasoning: Option<String>,
    pub model_used: Option<String>,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub previous_version_id: Option<Uuid>,
}

pub struct AnalysisRepo;

impl AnalysisRepo {
    pub async fn create(
        pool: &PgPool,
        record: &NewAnalysisRecord,
    ) -> Result<AnalysisRecordRow, sqlx::Error> {
        sqlx::query_as::<_, AnalysisRecordRow>(
            r#"
            INSERT INTO analysis_records (
                package_version_id, analysis_type, static_scan, llm_result,
                diff_summary, verdict, risk_score, reasoning, model_used,
                prompt_tokens, completion_tokens, previous_version_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING *
            "#,
        )
        .bind(record.package_version_id)
        .bind(&record.analysis_type)
        .bind(&record.static_scan)
        .bind(&record.llm_result)
        .bind(&record.diff_summary)
        .bind(&record.verdict)
        .bind(record.risk_score)
        .bind(&record.reasoning)
        .bind(&record.model_used)
        .bind(record.prompt_tokens)
        .bind(record.completion_tokens)
        .bind(record.previous_version_id)
        .fetch_one(pool)
        .await
    }

    pub async fn find_by_version(
        pool: &PgPool,
        package_version_id: Uuid,
    ) -> Result<Vec<AnalysisRecordRow>, sqlx::Error> {
        sqlx::query_as::<_, AnalysisRecordRow>(
            "SELECT * FROM analysis_records WHERE package_version_id = $1 ORDER BY analyzed_at DESC",
        )
        .bind(package_version_id)
        .fetch_all(pool)
        .await
    }

    pub async fn find_latest_by_version(
        pool: &PgPool,
        package_version_id: Uuid,
    ) -> Result<Option<AnalysisRecordRow>, sqlx::Error> {
        sqlx::query_as::<_, AnalysisRecordRow>(
            "SELECT * FROM analysis_records WHERE package_version_id = $1 ORDER BY analyzed_at DESC LIMIT 1",
        )
        .bind(package_version_id)
        .fetch_optional(pool)
        .await
    }
}
