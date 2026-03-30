use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use sp_core::{AnalysisStatus, Ecosystem};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PackageRow {
    pub id: Uuid,
    pub ecosystem: Ecosystem,
    pub name: String,
    pub normalized_name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PackageVersionRow {
    pub id: Uuid,
    pub package_id: Uuid,
    pub version: String,
    pub source_sha256: Option<String>,
    pub status: AnalysisStatus,
    pub error_message: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AnalysisRecordRow {
    pub id: Uuid,
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
    pub analyzed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AuditLogRow {
    pub id: Uuid,
    pub actor: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<Uuid>,
    pub details_json: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}
