use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Supported package ecosystems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "ecosystem", rename_all = "lowercase")]
pub enum Ecosystem {
    PyPI,
    Npm,
    Cargo,
    Go,
}

impl std::fmt::Display for Ecosystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PyPI => write!(f, "pypi"),
            Self::Npm => write!(f, "npm"),
            Self::Cargo => write!(f, "cargo"),
            Self::Go => write!(f, "go"),
        }
    }
}

/// Identifies a package within an ecosystem.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageId {
    pub ecosystem: Ecosystem,
    pub name: String,
}

/// Identifies a specific version of a package.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageVersion {
    pub package: PackageId,
    pub version: String,
}

/// A fetched and extracted source archive, ready for analysis.
/// The `_keep_alive` field holds a reference to the temp directory backing
/// `extracted_path`. When this struct is dropped, the temp dir is cleaned up.
pub struct SourceArchive {
    pub package_version: PackageVersion,
    pub archive_path: PathBuf,
    pub extracted_path: PathBuf,
    pub sha256: String,
    /// Prevents the temp directory from being deleted while this struct exists.
    /// Opaque — callers don't interact with it.
    pub _keep_alive: Option<std::sync::Arc<tempfile::TempDir>>,
}

impl std::fmt::Debug for SourceArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceArchive")
            .field("package_version", &self.package_version)
            .field("archive_path", &self.archive_path)
            .field("extracted_path", &self.extracted_path)
            .field("sha256", &self.sha256)
            .finish()
    }
}

/// Severity of a scan finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Info => write!(f, "info"),
            Self::Low => write!(f, "low"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

/// A single finding from the static regex scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanFinding {
    pub rule_id: String,
    pub severity: Severity,
    pub file_path: String,
    pub line: u32,
    pub description: String,
}

/// Result of the static scanner over a source directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticScanResult {
    pub findings: Vec<ScanFinding>,
    pub scanned_files: usize,
    pub skipped_files: usize,
}

/// A single finding reported by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityFlag {
    pub severity: Severity,
    pub file_path: String,
    pub line_range: String,
    pub description: String,
    pub confidence: f32,
    pub category: String,
}

/// Result of the LLM analysis (Gemini Flash).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmAnalysisResult {
    pub verdict: AnalysisVerdict,
    pub risk_score: f32,
    pub reasoning: String,
    pub flags: Vec<SecurityFlag>,
    pub model_used: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

/// A diff between two files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub unified_diff: String,
    pub lines_added: usize,
    pub lines_removed: usize,
}

/// Summary of differences between two source trees.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSummary {
    pub files_added: Vec<String>,
    pub files_removed: Vec<String>,
    pub files_modified: Vec<FileDiff>,
    pub total_lines_added: usize,
    pub total_lines_removed: usize,
}

/// The verdict of an analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnalysisVerdict {
    Approved,
    Rejected { reasons: Vec<String> },
    NeedsReview { flags: Vec<String> },
}

/// Combined result of a full analysis (static + LLM).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub verdict: AnalysisVerdict,
    pub static_scan: StaticScanResult,
    pub llm_analysis: Option<LlmAnalysisResult>,
    pub diff_summary: Option<DiffSummary>,
    pub analysis_type: AnalysisType,
    pub analyzed_at: DateTime<Utc>,
}

/// Whether this was a first-time full scan or a diff-based update scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisType {
    FullScan,
    DiffReview,
}

impl std::fmt::Display for AnalysisType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FullScan => write!(f, "full_scan"),
            Self::DiffReview => write!(f, "diff_review"),
        }
    }
}

/// Status of a package version in the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "analysis_status", rename_all = "snake_case")]
pub enum AnalysisStatus {
    Pending,
    Analyzing,
    Approved,
    Rejected,
    NeedsReview,
    Failed,
}

impl std::fmt::Display for AnalysisStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Analyzing => write!(f, "analyzing"),
            Self::Approved => write!(f, "approved"),
            Self::Rejected => write!(f, "rejected"),
            Self::NeedsReview => write!(f, "needs_review"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_serde_roundtrip() {
        let eco = Ecosystem::PyPI;
        let json = serde_json::to_string(&eco).unwrap();
        let parsed: Ecosystem = serde_json::from_str(&json).unwrap();
        assert_eq!(eco, parsed);
    }

    #[test]
    fn package_version_serde_roundtrip() {
        let pv = PackageVersion {
            package: PackageId {
                ecosystem: Ecosystem::PyPI,
                name: "requests".to_string(),
            },
            version: "2.31.0".to_string(),
        };
        let json = serde_json::to_string(&pv).unwrap();
        let parsed: PackageVersion = serde_json::from_str(&json).unwrap();
        assert_eq!(pv, parsed);
    }

    #[test]
    fn analysis_verdict_serde_roundtrip() {
        let approved = AnalysisVerdict::Approved;
        let json = serde_json::to_string(&approved).unwrap();
        let parsed: AnalysisVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(approved, parsed);

        let rejected = AnalysisVerdict::Rejected {
            reasons: vec!["data exfiltration".to_string()],
        };
        let json = serde_json::to_string(&rejected).unwrap();
        let parsed: AnalysisVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(rejected, parsed);

        let review = AnalysisVerdict::NeedsReview {
            flags: vec!["suspicious network call".to_string()],
        };
        let json = serde_json::to_string(&review).unwrap();
        let parsed: AnalysisVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(review, parsed);
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Info < Severity::Low);
        assert!(Severity::Low < Severity::Medium);
        assert!(Severity::Medium < Severity::High);
        assert!(Severity::High < Severity::Critical);
    }

    #[test]
    fn analysis_status_serde_roundtrip() {
        for status in [
            AnalysisStatus::Pending,
            AnalysisStatus::Analyzing,
            AnalysisStatus::Approved,
            AnalysisStatus::Rejected,
            AnalysisStatus::NeedsReview,
            AnalysisStatus::Failed,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: AnalysisStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, parsed);
        }
    }

    #[test]
    fn analysis_type_serde_roundtrip() {
        let full = AnalysisType::FullScan;
        let json = serde_json::to_string(&full).unwrap();
        assert_eq!(json, "\"full_scan\"");
        let parsed: AnalysisType = serde_json::from_str(&json).unwrap();
        assert_eq!(full, parsed);
    }

    #[test]
    fn diff_summary_serde_roundtrip() {
        let diff = DiffSummary {
            files_added: vec!["new.py".to_string()],
            files_removed: vec!["old.py".to_string()],
            files_modified: vec![FileDiff {
                path: "main.py".to_string(),
                unified_diff: "@@ -1,3 +1,4 @@\n+import os".to_string(),
                lines_added: 1,
                lines_removed: 0,
            }],
            total_lines_added: 1,
            total_lines_removed: 0,
        };
        let json = serde_json::to_string(&diff).unwrap();
        let parsed: DiffSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(diff.files_added, parsed.files_added);
        assert_eq!(diff.files_modified.len(), parsed.files_modified.len());
    }
}
