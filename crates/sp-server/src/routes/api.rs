use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use apalis_core::storage::Storage;
use sp_core::{AnalysisStatus, Ecosystem};
use sp_db::repos::{AnalysisRepo, AuditRepo, PackageRepo, VersionRepo};

use crate::jobs::AnalyzeJob;
use crate::state::AppState;

// ── Request/Response types ──

#[derive(Debug, Deserialize)]
pub struct PackageCheckRequest {
    pub name: String,
    pub version: String,
    #[serde(default = "default_ecosystem")]
    pub ecosystem: String,
}

fn default_ecosystem() -> String {
    "pypi".to_string()
}

#[derive(Debug, Serialize)]
pub struct PackageCheckResponse {
    pub name: String,
    pub version: String,
    pub ecosystem: String,
    pub version_id: Option<Uuid>,
    pub status: String,
    pub risk_score: Option<f32>,
    pub verdict: Option<String>,
    pub reasoning: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AnalysisDetailResponse {
    pub version_id: Uuid,
    pub package: String,
    pub version: String,
    pub ecosystem: String,
    pub status: String,
    pub analysis: Option<AnalysisDetail>,
}

#[derive(Debug, Serialize)]
pub struct AnalysisDetail {
    pub analysis_type: String,
    pub verdict: String,
    pub risk_score: Option<f32>,
    pub reasoning: Option<String>,
    pub model_used: Option<String>,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub static_scan: Option<serde_json::Value>,
    pub llm_result: Option<serde_json::Value>,
    pub diff_summary: Option<serde_json::Value>,
    pub analyzed_at: String,
}

#[derive(Debug, Deserialize)]
pub struct OverrideRequest {
    pub verdict: String,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

// ── Route handlers ──

async fn check_packages(
    State(state): State<AppState>,
    Json(packages): Json<Vec<PackageCheckRequest>>,
) -> Result<Json<Vec<PackageCheckResponse>>, StatusCode> {
    let mut results = Vec::with_capacity(packages.len());

    for req in &packages {
        if req.name.is_empty() || req.version.is_empty() {
            return Err(StatusCode::BAD_REQUEST);
        }
        let ecosystem = parse_ecosystem(&req.ecosystem)?;
        let normalized = sp_registry_pypi::normalize_name(&req.name);

        let pkg = PackageRepo::find_or_create(&state.db, ecosystem, &req.name, &normalized)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let version = VersionRepo::find_or_create_pending(&state.db, pkg.id, &req.version)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // If pending, enqueue analysis job (dedup: find_or_create_pending already
        // ensures we don't create duplicate versions)
        if version.status == AnalysisStatus::Pending {
            let job = AnalyzeJob {
                package_name: req.name.clone(),
                version: req.version.clone(),
                ecosystem: req.ecosystem.clone(),
                package_id: pkg.id.to_string(),
                version_id: version.id.to_string(),
            };
            let mut storage = state.job_storage.clone();
            if let Err(e) = storage.push(job).await {
                tracing::warn!(error = %e, "Failed to enqueue analysis job");
            }
        }

        let mut response = PackageCheckResponse {
            name: req.name.clone(),
            version: req.version.clone(),
            ecosystem: req.ecosystem.clone(),
            version_id: Some(version.id),
            status: version.status.to_string(),
            risk_score: None,
            verdict: None,
            reasoning: None,
            error: version.error_message.clone(),
        };

        // If analysis exists, include summary
        if matches!(
            version.status,
            AnalysisStatus::Approved | AnalysisStatus::Rejected | AnalysisStatus::NeedsReview
        ) && let Ok(Some(record)) =
            AnalysisRepo::find_latest_by_version(&state.db, version.id).await
        {
            response.risk_score = record.risk_score;
            response.verdict = Some(record.verdict.clone());
            response.reasoning = record.reasoning.clone();
        }

        results.push(response);
    }

    Ok(Json(results))
}

async fn list_packages(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let packages = PackageRepo::list_all(&state.db, Ecosystem::PyPI, params.limit, params.offset)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "packages": packages,
        "limit": params.limit,
        "offset": params.offset,
    })))
}

async fn list_versions(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let normalized = sp_registry_pypi::normalize_name(&name);

    let pkg = PackageRepo::find_by_name(&state.db, Ecosystem::PyPI, &normalized)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let versions = VersionRepo::list_by_package(&state.db, pkg.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "package": name,
        "versions": versions,
    })))
}

async fn get_analysis_details(
    State(state): State<AppState>,
    Path((name, version)): Path<(String, String)>,
) -> Result<Json<AnalysisDetailResponse>, StatusCode> {
    let normalized = sp_registry_pypi::normalize_name(&name);

    let pkg = PackageRepo::find_by_name(&state.db, Ecosystem::PyPI, &normalized)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let ver = VersionRepo::find(&state.db, pkg.id, &version)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let analysis = AnalysisRepo::find_latest_by_version(&state.db, ver.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(|r| AnalysisDetail {
            analysis_type: r.analysis_type,
            verdict: r.verdict,
            risk_score: r.risk_score,
            reasoning: r.reasoning,
            model_used: r.model_used,
            prompt_tokens: r.prompt_tokens,
            completion_tokens: r.completion_tokens,
            static_scan: r.static_scan,
            llm_result: r.llm_result,
            diff_summary: r.diff_summary,
            analyzed_at: r.analyzed_at.to_rfc3339(),
        });

    Ok(Json(AnalysisDetailResponse {
        version_id: ver.id,
        package: name,
        version,
        ecosystem: "pypi".to_string(),
        status: ver.status.to_string(),
        analysis,
    }))
}

async fn override_verdict(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((name, version)): Path<(String, String)>,
    Json(body): Json<OverrideRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Check admin token — reject if not configured or doesn't match
    let expected = &state.config.server.admin_token;
    if expected.is_empty() {
        tracing::warn!("Admin token not configured — override endpoint disabled");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    // Constant-time comparison to prevent timing attacks
    if provided.len() != expected.len()
        || !provided
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let normalized = sp_registry_pypi::normalize_name(&name);

    let pkg = PackageRepo::find_by_name(&state.db, Ecosystem::PyPI, &normalized)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let ver = VersionRepo::find(&state.db, pkg.id, &version)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let new_status = match body.verdict.as_str() {
        "approved" => AnalysisStatus::Approved,
        "rejected" => AnalysisStatus::Rejected,
        "needs_review" => AnalysisStatus::NeedsReview,
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    VersionRepo::update_status(&state.db, ver.id, new_status)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    AuditRepo::create(
        &state.db,
        "admin",
        "override_verdict",
        Some("package_version"),
        Some(ver.id),
        Some(serde_json::json!({
            "from": ver.status.to_string(),
            "to": body.verdict,
            "reason": body.reason,
        })),
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "version_id": ver.id,
        "new_status": body.verdict,
    })))
}

// ── Helpers ──

fn parse_ecosystem(s: &str) -> Result<Ecosystem, StatusCode> {
    match s {
        "pypi" => Ok(Ecosystem::PyPI),
        "npm" => Ok(Ecosystem::Npm),
        "cargo" => Ok(Ecosystem::Cargo),
        "go" => Ok(Ecosystem::Go),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

// ── Router ──

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/packages/check", post(check_packages))
        .route("/packages", get(list_packages))
        .route("/packages/{name}/versions", get(list_versions))
        .route(
            "/packages/{name}/versions/{version}",
            get(get_analysis_details),
        )
        .route(
            "/packages/{name}/versions/{version}/override",
            post(override_verdict),
        )
}
