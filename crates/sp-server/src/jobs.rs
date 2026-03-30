use std::path::PathBuf;

use apalis::prelude::*;
use apalis_sql::context::SqlContext;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use sp_analysis::{AnalysisConfig, AnalysisOrchestrator, GeminiRunner};
use sp_core::{
    AnalysisStatus, AnalysisVerdict, Ecosystem, PackageId, PackageVersion, RegistryClient,
};
use sp_db::repos::{AnalysisRepo, NewAnalysisRecord, VersionRepo};
use sp_registry_pypi::{PyPIRegistryClient, SourceCache};

use crate::state::AppState;

/// The job type pushed to the apalis queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzeJob {
    pub package_name: String,
    pub version: String,
    pub ecosystem: String,
    pub package_id: String,
    pub version_id: String,
}

/// Handle an analysis job — decides full scan vs diff scan based on DB state.
pub async fn handle_analyze(
    job: AnalyzeJob,
    data: Data<AppState>,
    _ctx: SqlContext,
) -> Result<(), Error> {
    info!(
        package = %job.package_name,
        version = %job.version,
        "Starting analysis"
    );

    let version_id: uuid::Uuid = job.version_id.parse().map_err(|e| {
        let err: Box<dyn std::error::Error + Send + Sync> = format!("bad version_id: {e}").into();
        Error::Abort(std::sync::Arc::new(err))
    })?;

    // Update status to analyzing
    VersionRepo::update_status(&data.db, version_id, AnalysisStatus::Analyzing)
        .await
        .map_err(to_failed)?;

    match run_analysis(&job, &data).await {
        Ok((status, record)) => {
            AnalysisRepo::create(&data.db, &record)
                .await
                .map_err(to_failed)?;

            VersionRepo::update_status(&data.db, version_id, status)
                .await
                .map_err(to_failed)?;

            info!(
                package = %job.package_name,
                version = %job.version,
                status = %status,
                "Analysis complete"
            );
            Ok(())
        }
        Err(e) => {
            let error_msg = e.to_string();
            error!(
                package = %job.package_name,
                version = %job.version,
                error = %error_msg,
                "Analysis failed"
            );
            let _ = VersionRepo::update_status_with_error(
                &data.db,
                version_id,
                AnalysisStatus::Failed,
                &error_msg,
            )
            .await;
            Err(to_failed(e))
        }
    }
}

fn to_failed(e: impl std::error::Error + Send + Sync + 'static) -> Error {
    let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(e);
    Error::Failed(std::sync::Arc::new(boxed))
}

async fn run_analysis(
    job: &AnalyzeJob,
    state: &AppState,
) -> Result<(AnalysisStatus, NewAnalysisRecord), sp_core::SpError> {
    let config = &state.config;

    let skills_dir = PathBuf::from("skills/pypi");
    let gemini = GeminiRunner::new(
        &config.analysis.gemini_model,
        &skills_dir,
        config.analysis.gemini_timeout_seconds,
        config.analysis.gemini_binary.as_deref(),
    );

    let orchestrator = AnalysisOrchestrator::new(
        gemini,
        AnalysisConfig {
            max_source_size_bytes: config.analysis.max_source_size_bytes,
        },
    );

    let cache = SourceCache::new(&config.cache.source_cache_dir);
    let pypi_client = PyPIRegistryClient::new(
        &config.pypi.upstream_index,
        &config.pypi.upstream_json_api,
        cache,
    );

    let pv = PackageVersion {
        package: PackageId {
            ecosystem: Ecosystem::PyPI,
            name: job.package_name.clone(),
        },
        version: job.version.clone(),
    };

    let source = pypi_client.fetch_source(&pv).await?;

    let version_id: uuid::Uuid = job
        .version_id
        .parse()
        .map_err(|e| sp_core::SpError::Other(format!("bad version_id: {e}")))?;

    let package_id: uuid::Uuid = job
        .package_id
        .parse()
        .map_err(|e| sp_core::SpError::Other(format!("bad package_id: {e}")))?;

    // Check for previous approved version to decide full scan vs diff review
    let approved_versions = sp_db::repos::VersionRepo::list_approved(&state.db, package_id)
        .await
        .unwrap_or_default();

    let result = if let Some(prev_version) = find_previous_version(&approved_versions, &job.version)
    {
        // Diff review: fetch old source, create synthetic git repo, run diff skill
        let prev_pv = PackageVersion {
            package: pv.package.clone(),
            version: prev_version.clone(),
        };
        let cache2 = SourceCache::new(&config.cache.source_cache_dir);
        let pypi_client2 = PyPIRegistryClient::new(
            &config.pypi.upstream_index,
            &config.pypi.upstream_json_api,
            cache2,
        );

        match pypi_client2.fetch_source(&prev_pv).await {
            Ok(old_source) => {
                match setup_diff_repo(&old_source.extracted_path, &source.extracted_path) {
                    Ok(repo_dir) => {
                        info!(
                            old_version = %prev_version,
                            new_version = %job.version,
                            "Running diff review"
                        );
                        orchestrator
                            .analyze_update(&repo_dir, "HEAD~1", "HEAD")
                            .await?
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Diff repo setup failed, falling back to full scan");
                        orchestrator.analyze_new(&source.extracted_path).await?
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, prev = %prev_version, "Could not fetch previous version, falling back to full scan");
                orchestrator.analyze_new(&source.extracted_path).await?
            }
        }
    } else {
        orchestrator.analyze_new(&source.extracted_path).await?
    };

    let status = match &result.verdict {
        AnalysisVerdict::Approved => AnalysisStatus::Approved,
        AnalysisVerdict::Rejected { .. } => AnalysisStatus::Rejected,
        AnalysisVerdict::NeedsReview { .. } => AnalysisStatus::NeedsReview,
    };

    let record = NewAnalysisRecord {
        package_version_id: version_id,
        analysis_type: result.analysis_type.to_string(),
        static_scan: serde_json::to_value(&result.static_scan).ok(),
        llm_result: result
            .llm_analysis
            .as_ref()
            .and_then(|a| serde_json::to_value(a).ok()),
        diff_summary: result
            .diff_summary
            .as_ref()
            .and_then(|d| serde_json::to_value(d).ok()),
        verdict: match &result.verdict {
            AnalysisVerdict::Approved => "approved".to_string(),
            AnalysisVerdict::Rejected { .. } => "rejected".to_string(),
            AnalysisVerdict::NeedsReview { .. } => "needs_review".to_string(),
        },
        risk_score: result.llm_analysis.as_ref().map(|a| a.risk_score),
        reasoning: result.llm_analysis.as_ref().map(|a| a.reasoning.clone()),
        model_used: result.llm_analysis.as_ref().map(|a| a.model_used.clone()),
        prompt_tokens: result.llm_analysis.as_ref().map(|a| a.prompt_tokens as i32),
        completion_tokens: result
            .llm_analysis
            .as_ref()
            .map(|a| a.completion_tokens as i32),
        previous_version_id: None,
    };

    Ok((status, record))
}

/// Find the most recent approved version that comes before the given version
/// in PEP 440 ordering.
fn find_previous_version(
    approved: &[sp_db::PackageVersionRow],
    current_version: &str,
) -> Option<String> {
    use sp_registry_pypi::client::compare_pep440;

    let mut candidates: Vec<&str> = approved
        .iter()
        .map(|v| v.version.as_str())
        .filter(|v| compare_pep440(v, current_version) == std::cmp::Ordering::Less)
        .collect();

    candidates.sort_by(|a, b| compare_pep440(a, b));
    candidates.last().map(|v| v.to_string())
}

/// Create a synthetic git repo with old source as first commit, new source as second.
/// This lets the diff-security-review skill use `git diff HEAD~1..HEAD`.
fn setup_diff_repo(
    old_source: &std::path::Path,
    new_source: &std::path::Path,
) -> Result<std::path::PathBuf, sp_core::SpError> {
    let tmp = tempfile::tempdir().map_err(|e| sp_core::SpError::Other(format!("tmpdir: {e}")))?;
    let repo_dir = tmp.path().to_path_buf();
    // Keep the dir alive — don't let it be deleted on drop.
    // Caller is responsible for cleanup (or it persists until process exit).
    std::mem::forget(tmp);

    // Init repo
    run_git(&repo_dir, &["init"])?;
    run_git(&repo_dir, &["config", "user.email", "sp@localhost"])?;
    run_git(&repo_dir, &["config", "user.name", "secure-packages"])?;

    // Copy old source → commit as "old version"
    copy_dir_contents(old_source, &repo_dir)?;
    run_git(&repo_dir, &["add", "-A"])?;
    run_git(&repo_dir, &["commit", "-m", "old version", "--allow-empty"])?;

    // Remove all tracked files, copy new source → commit as "new version"
    // Use git rm to track deletions properly
    run_git(&repo_dir, &["rm", "-rf", "--quiet", "."])?;
    copy_dir_contents(new_source, &repo_dir)?;
    run_git(&repo_dir, &["add", "-A"])?;
    run_git(&repo_dir, &["commit", "-m", "new version", "--allow-empty"])?;

    Ok(repo_dir)
}

fn run_git(dir: &std::path::Path, args: &[&str]) -> Result<(), sp_core::SpError> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| sp_core::SpError::Other(format!("git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(sp_core::SpError::Other(format!(
            "git {} failed: {stderr}",
            args.join(" ")
        )));
    }
    Ok(())
}

fn copy_dir_contents(src: &std::path::Path, dst: &std::path::Path) -> Result<(), sp_core::SpError> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            // Skip .git directories
            if entry.file_name() == ".git" {
                continue;
            }
            std::fs::create_dir_all(&dst_path)?;
            copy_dir_contents(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
