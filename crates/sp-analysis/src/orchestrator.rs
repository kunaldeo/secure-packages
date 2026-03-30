use std::path::Path;

use chrono::Utc;
use tracing::info;

use sp_core::SpError;
use sp_core::{
    AnalysisResult, AnalysisType, AnalysisVerdict, DiffSummary, FileDiff, LlmAnalysisResult,
    SecurityFlag, Severity, StaticScanResult,
};

use crate::gemini::{GeminiResult, GeminiRunner};

pub struct AnalysisConfig {
    pub max_source_size_bytes: u64,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            max_source_size_bytes: 50 * 1024 * 1024, // 50 MB
        }
    }
}

pub struct AnalysisOrchestrator {
    gemini: GeminiRunner,
    config: AnalysisConfig,
}

impl AnalysisOrchestrator {
    pub fn new(gemini: GeminiRunner, config: AnalysisConfig) -> Self {
        Self { gemini, config }
    }

    /// Full audit for a first-time package.
    pub async fn analyze_new(&self, source_dir: &Path) -> Result<AnalysisResult, SpError> {
        let size = dir_size(source_dir)?;
        if size > self.config.max_source_size_bytes {
            info!(
                size_bytes = size,
                limit = self.config.max_source_size_bytes,
                "Source too large for automated analysis"
            );
            return Ok(AnalysisResult {
                verdict: AnalysisVerdict::NeedsReview {
                    flags: vec![format!(
                        "Source size ({} bytes) exceeds limit ({} bytes)",
                        size, self.config.max_source_size_bytes
                    )],
                },
                static_scan: StaticScanResult {
                    findings: vec![],
                    scanned_files: 0,
                    skipped_files: 0,
                },
                llm_analysis: None,
                diff_summary: None,
                analysis_type: AnalysisType::FullScan,
                analyzed_at: Utc::now(),
            });
        }

        let result = self.gemini.run_full_audit(source_dir).await?;
        Ok(map_to_analysis_result(
            result,
            AnalysisType::FullScan,
            self.gemini.model_name(),
        ))
    }

    /// Diff-based review for a version update.
    pub async fn analyze_update(
        &self,
        repo_dir: &Path,
        from_commit: &str,
        to_commit: &str,
    ) -> Result<AnalysisResult, SpError> {
        let result = self
            .gemini
            .run_diff_review(repo_dir, from_commit, to_commit)
            .await?;
        Ok(map_to_analysis_result(
            result,
            AnalysisType::DiffReview,
            self.gemini.model_name(),
        ))
    }
}

fn map_to_analysis_result(
    gemini: GeminiResult,
    analysis_type: AnalysisType,
    model_name: &str,
) -> AnalysisResult {
    let verdict = match gemini.report.verdict.as_str() {
        "approved" => AnalysisVerdict::Approved,
        "rejected" => AnalysisVerdict::Rejected {
            reasons: gemini
                .report
                .findings
                .iter()
                .filter(|f| f.severity == "critical" || f.severity == "high")
                .map(|f| f.description.clone())
                .collect(),
        },
        "needs_review" => AnalysisVerdict::NeedsReview {
            flags: gemini
                .report
                .findings
                .iter()
                .filter(|f| f.severity == "medium" || f.severity == "high")
                .map(|f| f.description.clone())
                .collect(),
        },
        _ => AnalysisVerdict::NeedsReview {
            flags: vec![format!("Unknown verdict: {}", gemini.report.verdict)],
        },
    };

    let flags: Vec<SecurityFlag> = gemini
        .report
        .findings
        .iter()
        .map(|f| SecurityFlag {
            severity: match f.severity.as_str() {
                "critical" => Severity::Critical,
                "high" => Severity::High,
                "medium" => Severity::Medium,
                "low" => Severity::Low,
                _ => Severity::Info,
            },
            file_path: f.file_path.clone(),
            line_range: f.line_range.clone().unwrap_or_default(),
            description: f.description.clone(),
            confidence: f.confidence.unwrap_or(0.0),
            category: f.category.clone().unwrap_or_else(|| "other".to_string()),
        })
        .collect();

    let llm_analysis = LlmAnalysisResult {
        verdict: verdict.clone(),
        risk_score: gemini.report.risk_score,
        reasoning: gemini.report.reasoning.clone().unwrap_or_default(),
        flags,
        model_used: model_name.to_string(),
        prompt_tokens: gemini.stats.input_tokens,
        completion_tokens: gemini.stats.output_tokens,
    };

    let diff_summary = if analysis_type == AnalysisType::DiffReview {
        let files = gemini
            .report
            .files_changed
            .as_ref()
            .cloned()
            .unwrap_or_default();
        Some(DiffSummary {
            files_added: vec![],
            files_removed: vec![],
            files_modified: files
                .into_iter()
                .map(|path| FileDiff {
                    path,
                    unified_diff: String::new(),
                    lines_added: 0,
                    lines_removed: 0,
                })
                .collect(),
            total_lines_added: 0,
            total_lines_removed: 0,
        })
    } else {
        None
    };

    AnalysisResult {
        verdict,
        static_scan: StaticScanResult {
            findings: vec![],
            scanned_files: 0,
            skipped_files: 0,
        },
        llm_analysis: Some(llm_analysis),
        diff_summary,
        analysis_type,
        analyzed_at: Utc::now(),
    }
}

/// Compute total size of a directory tree in bytes.
fn dir_size(path: &Path) -> Result<u64, SpError> {
    let mut total = 0u64;
    if path.is_file() {
        return Ok(path.metadata()?.len());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            total += dir_size(&entry.path())?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gemini::{GeminiFinding, GeminiReport, GeminiStats};

    fn make_gemini_result(
        verdict: &str,
        risk_score: f32,
        findings: Vec<GeminiFinding>,
    ) -> GeminiResult {
        GeminiResult {
            report: GeminiReport {
                verdict: verdict.to_string(),
                risk_score,
                reasoning: Some("Test reasoning".to_string()),
                files_reviewed: Some(vec!["test.py".to_string()]),
                files_skipped: None,
                grep_hits: None,
                commit_range: None,
                commits_reviewed: None,
                files_changed: None,
                findings,
            },
            stats: GeminiStats::default(),
        }
    }

    #[test]
    fn test_map_approved() {
        let result = map_to_analysis_result(
            make_gemini_result("approved", 0.05, vec![]),
            AnalysisType::FullScan,
            "test-model",
        );
        assert_eq!(result.verdict, AnalysisVerdict::Approved);
        assert_eq!(result.analysis_type, AnalysisType::FullScan);
        assert!(result.diff_summary.is_none());
    }

    #[test]
    fn test_map_rejected() {
        let result = map_to_analysis_result(
            make_gemini_result(
                "rejected",
                0.95,
                vec![GeminiFinding {
                    severity: "critical".to_string(),
                    file_path: "setup.py".to_string(),
                    line_range: Some("1-5".to_string()),
                    description: "Data exfiltration".to_string(),
                    confidence: Some(1.0),
                    category: Some("exfiltration".to_string()),
                    change_type: None,
                }],
            ),
            AnalysisType::FullScan,
            "test-model",
        );
        match &result.verdict {
            AnalysisVerdict::Rejected { reasons } => {
                assert_eq!(reasons.len(), 1);
                assert_eq!(reasons[0], "Data exfiltration");
            }
            _ => panic!("Expected Rejected verdict"),
        }
    }

    #[test]
    fn test_map_needs_review() {
        let result = map_to_analysis_result(
            make_gemini_result(
                "needs_review",
                0.5,
                vec![GeminiFinding {
                    severity: "medium".to_string(),
                    file_path: "src/main.py".to_string(),
                    line_range: None,
                    description: "Suspicious pattern".to_string(),
                    confidence: Some(0.6),
                    category: None,
                    change_type: None,
                }],
            ),
            AnalysisType::FullScan,
            "test-model",
        );
        assert!(matches!(
            result.verdict,
            AnalysisVerdict::NeedsReview { .. }
        ));
    }

    #[test]
    fn test_map_diff_review_has_diff_summary() {
        let result = map_to_analysis_result(
            make_gemini_result("approved", 0.0, vec![]),
            AnalysisType::DiffReview,
            "test-model",
        );
        assert!(result.diff_summary.is_some());
        assert_eq!(result.analysis_type, AnalysisType::DiffReview);
    }

    #[test]
    fn test_map_unknown_verdict() {
        let result = map_to_analysis_result(
            make_gemini_result("something_weird", 0.5, vec![]),
            AnalysisType::FullScan,
            "test-model",
        );
        match &result.verdict {
            AnalysisVerdict::NeedsReview { flags } => {
                assert!(flags[0].contains("Unknown verdict"));
            }
            _ => panic!("Expected NeedsReview for unknown verdict"),
        }
    }

    #[test]
    fn test_severity_mapping() {
        let findings = vec![
            GeminiFinding {
                severity: "critical".to_string(),
                file_path: "a.py".to_string(),
                line_range: None,
                description: "a".to_string(),
                confidence: None,
                category: None,
                change_type: None,
            },
            GeminiFinding {
                severity: "high".to_string(),
                file_path: "b.py".to_string(),
                line_range: None,
                description: "b".to_string(),
                confidence: None,
                category: None,
                change_type: None,
            },
            GeminiFinding {
                severity: "medium".to_string(),
                file_path: "c.py".to_string(),
                line_range: None,
                description: "c".to_string(),
                confidence: None,
                category: None,
                change_type: None,
            },
            GeminiFinding {
                severity: "low".to_string(),
                file_path: "d.py".to_string(),
                line_range: None,
                description: "d".to_string(),
                confidence: None,
                category: None,
                change_type: None,
            },
            GeminiFinding {
                severity: "info".to_string(),
                file_path: "e.py".to_string(),
                line_range: None,
                description: "e".to_string(),
                confidence: None,
                category: None,
                change_type: None,
            },
        ];
        let result = map_to_analysis_result(
            make_gemini_result("rejected", 0.9, findings),
            AnalysisType::FullScan,
            "test-model",
        );
        let llm = result.llm_analysis.unwrap();
        assert_eq!(llm.flags[0].severity, Severity::Critical);
        assert_eq!(llm.flags[1].severity, Severity::High);
        assert_eq!(llm.flags[2].severity, Severity::Medium);
        assert_eq!(llm.flags[3].severity, Severity::Low);
        assert_eq!(llm.flags[4].severity, Severity::Info);
    }

    #[test]
    fn test_dir_size() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("b.txt"), "world!").unwrap();
        let size = dir_size(dir.path()).unwrap();
        assert_eq!(size, 11); // 5 + 6
    }
}
