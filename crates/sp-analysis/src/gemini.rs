use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::{debug, info, warn};

use sp_core::SpError;

/// Report returned by the Gemini CLI after analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiReport {
    pub verdict: String,
    pub risk_score: f32,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub files_reviewed: Option<Vec<String>>,
    #[serde(default)]
    pub files_skipped: Option<Vec<String>>,
    #[serde(default)]
    pub grep_hits: Option<u32>,
    #[serde(default)]
    pub commit_range: Option<String>,
    #[serde(default)]
    pub commits_reviewed: Option<Vec<CommitInfo>>,
    #[serde(default)]
    pub files_changed: Option<Vec<String>>,
    #[serde(default)]
    pub findings: Vec<GeminiFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub sha: String,
    pub message: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiFinding {
    pub severity: String,
    pub file_path: String,
    #[serde(default)]
    pub line_range: Option<String>,
    pub description: String,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub change_type: Option<String>,
}

/// Token and latency stats from the Gemini CLI output.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiStats {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_latency_ms: u64,
    pub total_tool_calls: u32,
}

/// Full parsed output from the Gemini CLI JSON mode.
#[derive(Debug, Clone, Deserialize)]
struct GeminiCliOutput {
    response: String,
    #[serde(default)]
    stats: Option<serde_json::Value>,
}

/// Result of a Gemini CLI invocation.
#[derive(Debug, Clone)]
pub struct GeminiResult {
    pub report: GeminiReport,
    pub stats: GeminiStats,
}

pub struct GeminiRunner {
    model: String,
    binary: GeminiBinary,
    skills_dir: PathBuf,
    timeout: Duration,
}

enum GeminiBinary {
    System(PathBuf),
    Npx,
}

impl GeminiRunner {
    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// Create a new runner. Locates the gemini binary on PATH, falls back to npx.
    pub fn new(
        model: impl Into<String>,
        skills_dir: impl Into<PathBuf>,
        timeout_seconds: u64,
        gemini_binary_override: Option<&str>,
    ) -> Self {
        let binary = if let Some(path) = gemini_binary_override {
            GeminiBinary::System(PathBuf::from(path))
        } else {
            match which_gemini() {
                Some(path) => {
                    info!(path = %path.display(), "Found gemini binary on PATH");
                    GeminiBinary::System(path)
                }
                None => {
                    info!("gemini not found on PATH, will use npx");
                    GeminiBinary::Npx
                }
            }
        };

        Self {
            model: model.into(),
            binary,
            skills_dir: skills_dir.into(),
            timeout: Duration::from_secs(timeout_seconds),
        }
    }

    /// Run a full security audit on a package source directory.
    pub async fn run_full_audit(&self, source_dir: &Path) -> Result<GeminiResult, SpError> {
        self.install_skill(source_dir, "security-audit")?;

        let prompt = "Perform a supply chain security audit of this Python package and certify it.";

        self.run_gemini(source_dir, prompt).await
    }

    /// Run a diff-based security review between two commits.
    pub async fn run_diff_review(
        &self,
        repo_dir: &Path,
        from_commit: &str,
        to_commit: &str,
    ) -> Result<GeminiResult, SpError> {
        self.install_skill(repo_dir, "diff-security-review")?;

        let prompt = format!(
            "Review the changes from {} to {} for security risks.",
            from_commit, to_commit
        );

        self.run_gemini(repo_dir, &prompt).await
    }

    /// Copy a skill into the target directory's .gemini/skills/.
    fn install_skill(&self, target_dir: &Path, skill_name: &str) -> Result<(), SpError> {
        let src = self.skills_dir.join(skill_name);
        let dest = target_dir.join(".gemini").join("skills").join(skill_name);

        if !src.exists() {
            return Err(SpError::Other(format!(
                "Skill not found: {}",
                src.display()
            )));
        }

        std::fs::create_dir_all(&dest)?;

        // Copy SKILL.md
        let skill_file = src.join("SKILL.md");
        if skill_file.exists() {
            std::fs::copy(&skill_file, dest.join("SKILL.md"))?;
            debug!(skill = skill_name, dest = %dest.display(), "Installed skill");
        }

        Ok(())
    }

    /// Execute gemini CLI and parse output.
    async fn run_gemini(&self, working_dir: &Path, prompt: &str) -> Result<GeminiResult, SpError> {
        let mut cmd = match &self.binary {
            GeminiBinary::System(path) => Command::new(path),
            GeminiBinary::Npx => {
                let mut c = Command::new("npx");
                c.arg("@google/gemini-cli");
                c
            }
        };

        cmd.args(["--model", &self.model])
            .arg("-y")
            .args(["-p", prompt])
            .args(["--output-format", "json"])
            .current_dir(working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        info!(
            model = %self.model,
            dir = %working_dir.display(),
            "Running gemini CLI"
        );

        let output = tokio::time::timeout(self.timeout, cmd.output())
            .await
            .map_err(|_| {
                SpError::AnalysisFailed(format!(
                    "Gemini CLI timed out after {}s",
                    self.timeout.as_secs()
                ))
            })?
            .map_err(|e| SpError::AnalysisFailed(format!("Failed to execute gemini CLI: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output.status.code().unwrap_or(-1);
            return Err(SpError::AnalysisFailed(format!(
                "Gemini CLI exited with code {code}: {stderr}"
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_gemini_output(&stdout)
    }
}

/// Find the `gemini` binary on PATH.
fn which_gemini() -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("gemini");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Parse the full Gemini CLI JSON output into a GeminiResult.
fn parse_gemini_output(raw: &str) -> Result<GeminiResult, SpError> {
    // The CLI outputs JSON with {session_id, response, stats}.
    // The response field contains our report — possibly:
    //   1. Clean JSON
    //   2. JSON wrapped in ```json ... ``` fences
    //   3. Prose text with JSON embedded somewhere in it
    let cli_output: GeminiCliOutput = serde_json::from_str(raw)
        .map_err(|e| SpError::LlmParseFailed(format!("Failed to parse Gemini CLI output: {e}")))?;

    let report = extract_json_report(&cli_output.response)?;
    let stats = extract_stats(&cli_output.stats);

    Ok(GeminiResult { report, stats })
}

/// Try multiple strategies to extract a GeminiReport from a response string.
fn extract_json_report(response: &str) -> Result<GeminiReport, SpError> {
    // Try parsing with the raw response first, then with sanitized version
    for candidate in candidates_from_response(response) {
        if let Ok(report) = serde_json::from_str::<GeminiReport>(&candidate) {
            return Ok(report);
        }
    }

    warn!(response = %response, "Failed to parse Gemini report");
    Err(SpError::LlmParseFailed(format!(
        "Could not extract JSON report from response ({} chars)",
        response.len()
    )))
}

/// Generate candidate JSON strings to try parsing, in order of preference.
fn candidates_from_response(response: &str) -> Vec<String> {
    let mut candidates = Vec::new();

    // Strategy 1: Strip markdown fences
    let stripped = strip_markdown_fences(response);
    candidates.push(stripped.to_string());
    candidates.push(sanitize_json_escapes(stripped));

    // Strategy 2: Extract JSON object by brace matching
    if let Some(json_str) = extract_json_object(response) {
        candidates.push(json_str.to_string());
        candidates.push(sanitize_json_escapes(json_str));
    }

    // Strategy 3: Look for ```json blocks within the text
    for block in response.split("```json") {
        if let Some(end) = block.find("```") {
            let candidate = block[..end].trim();
            candidates.push(candidate.to_string());
            candidates.push(sanitize_json_escapes(candidate));
        }
    }

    candidates
}

/// Fix invalid JSON escape sequences that LLMs sometimes produce.
/// Converts invalid \x, \p, \d etc. to \\x, \\p, \\d (literal backslash).
fn sanitize_json_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut in_string = false;

    while let Some(c) = chars.next() {
        if c == '"' && !in_string {
            in_string = true;
            result.push(c);
        } else if c == '"' && in_string {
            in_string = false;
            result.push(c);
        } else if c == '\\' && in_string {
            match chars.peek() {
                // Valid JSON escapes: " \ / b f n r t u
                Some('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u') => {
                    result.push('\\');
                    result.push(chars.next().unwrap());
                }
                // Invalid escapes inside strings: \x, \p, \d, etc. — double the backslash
                Some(_) => {
                    result.push('\\');
                    result.push('\\');
                }
                None => {
                    result.push('\\');
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Find the outermost JSON object in a string by matching braces.
fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, c) in s[start..].char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        match c {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Strip markdown code fences (```json ... ```) if present.
pub fn strip_markdown_fences(s: &str) -> &str {
    let trimmed = s.trim();

    // Check for ```json or ``` at the start
    let without_opening = if let Some(rest) = trimmed.strip_prefix("```json") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest
    } else {
        return trimmed;
    };

    // Strip closing ```
    let without_closing = if let Some(rest) = without_opening.trim().strip_suffix("```") {
        rest
    } else {
        without_opening
    };

    without_closing.trim()
}

/// Extract stats from the Gemini CLI stats JSON.
fn extract_stats(stats_json: &Option<serde_json::Value>) -> GeminiStats {
    let Some(stats) = stats_json else {
        return GeminiStats::default();
    };

    let mut input_tokens = 0u32;
    let mut output_tokens = 0u32;
    let mut total_latency_ms = 0u64;
    let mut total_tool_calls = 0u32;

    // Navigate stats.models.*.{api, tokens}
    if let Some(models) = stats.get("models").and_then(|m| m.as_object()) {
        for model_stats in models.values() {
            if let Some(api) = model_stats.get("api") {
                total_latency_ms += api
                    .get("totalLatencyMs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
            }
            if let Some(tokens) = model_stats.get("tokens") {
                let input = tokens.get("input").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let total = tokens.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                // Also check for explicit "output" field
                let output = tokens
                    .get("output")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .unwrap_or_else(|| total.saturating_sub(input));
                input_tokens += input;
                output_tokens += output;
            }
        }
    }

    if let Some(tools) = stats.get("tools") {
        total_tool_calls = tools
            .get("totalCalls")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
    }

    GeminiStats {
        input_tokens,
        output_tokens,
        total_latency_ms,
        total_tool_calls,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_markdown_fences_with_json_fence() {
        let input = "```json\n{\"verdict\": \"approved\"}\n```";
        assert_eq!(strip_markdown_fences(input), "{\"verdict\": \"approved\"}");
    }

    #[test]
    fn test_strip_markdown_fences_with_plain_fence() {
        let input = "```\n{\"verdict\": \"approved\"}\n```";
        assert_eq!(strip_markdown_fences(input), "{\"verdict\": \"approved\"}");
    }

    #[test]
    fn test_strip_markdown_fences_no_fence() {
        let input = "{\"verdict\": \"approved\"}";
        assert_eq!(strip_markdown_fences(input), "{\"verdict\": \"approved\"}");
    }

    #[test]
    fn test_strip_markdown_fences_with_whitespace() {
        let input = "  ```json\n  {\"verdict\": \"approved\"}  \n```  ";
        assert_eq!(strip_markdown_fences(input), "{\"verdict\": \"approved\"}");
    }

    #[test]
    fn test_parse_gemini_output_clean_response() {
        let raw = r#"{
            "session_id": "test-123",
            "response": "{\"verdict\": \"approved\", \"risk_score\": 0.05, \"reasoning\": \"Clean\", \"findings\": []}",
            "stats": {
                "models": {
                    "gemini-3.1-flash-lite-preview": {
                        "api": {"totalRequests": 2, "totalErrors": 0, "totalLatencyMs": 3500},
                        "tokens": {"input": 10000, "total": 12000}
                    }
                },
                "tools": {"totalCalls": 5, "totalSuccess": 5}
            }
        }"#;

        let result = parse_gemini_output(raw).unwrap();
        assert_eq!(result.report.verdict, "approved");
        assert!((result.report.risk_score - 0.05).abs() < 0.001);
        assert_eq!(result.report.findings.len(), 0);
        assert_eq!(result.stats.input_tokens, 10000);
        assert_eq!(result.stats.output_tokens, 2000);
        assert_eq!(result.stats.total_latency_ms, 3500);
        assert_eq!(result.stats.total_tool_calls, 5);
    }

    #[test]
    fn test_parse_gemini_output_fenced_response() {
        let raw = r#"{
            "session_id": "test-456",
            "response": "```json\n{\"verdict\": \"rejected\", \"risk_score\": 0.95, \"reasoning\": \"Malicious\", \"findings\": [{\"severity\": \"critical\", \"file_path\": \"setup.py\", \"description\": \"Exfiltration\"}]}\n```",
            "stats": null
        }"#;

        let result = parse_gemini_output(raw).unwrap();
        assert_eq!(result.report.verdict, "rejected");
        assert_eq!(result.report.findings.len(), 1);
        assert_eq!(result.report.findings[0].severity, "critical");
    }

    #[test]
    fn test_parse_gemini_output_with_findings() {
        let raw = r#"{
            "session_id": "test-789",
            "response": "{\"verdict\": \"needs_review\", \"risk_score\": 0.5, \"reasoning\": \"Suspicious\", \"findings\": [{\"severity\": \"medium\", \"file_path\": \"src/main.py\", \"line_range\": \"10-15\", \"description\": \"Dynamic exec\", \"confidence\": 0.7, \"category\": \"dynamic_execution\"}], \"files_reviewed\": [\"src/main.py\", \"setup.py\"], \"grep_hits\": 3}"
        }"#;

        let result = parse_gemini_output(raw).unwrap();
        assert_eq!(result.report.verdict, "needs_review");
        assert_eq!(
            result.report.files_reviewed,
            Some(vec!["src/main.py".to_string(), "setup.py".to_string()])
        );
        assert_eq!(result.report.grep_hits, Some(3));
        assert_eq!(
            result.report.findings[0].category.as_deref(),
            Some("dynamic_execution")
        );
        assert_eq!(result.report.findings[0].confidence, Some(0.7));
    }

    #[test]
    fn test_parse_gemini_output_diff_review() {
        let raw = r#"{
            "session_id": "test-diff",
            "response": "{\"verdict\": \"rejected\", \"risk_score\": 0.9, \"reasoning\": \"Backdoor found\", \"commit_range\": \"abc..def\", \"commits_reviewed\": [{\"sha\": \"def456\", \"message\": \"Fix typo\", \"author\": \"attacker\", \"date\": \"2026-01-01\"}], \"files_changed\": [\"src/helpers.py\"], \"findings\": [{\"severity\": \"critical\", \"file_path\": \"src/helpers.py\", \"line_range\": \"27-46\", \"change_type\": \"added\", \"description\": \"Exfiltration\", \"confidence\": 1.0, \"category\": \"exfiltration\"}]}"
        }"#;

        let result = parse_gemini_output(raw).unwrap();
        assert_eq!(result.report.commit_range.as_deref(), Some("abc..def"));
        assert_eq!(result.report.commits_reviewed.as_ref().unwrap().len(), 1);
        assert_eq!(
            result.report.files_changed.as_ref().unwrap(),
            &["src/helpers.py"]
        );
        assert_eq!(
            result.report.findings[0].change_type.as_deref(),
            Some("added")
        );
    }

    #[test]
    fn test_parse_gemini_output_bad_json() {
        let raw = r#"{"session_id": "x", "response": "not json at all"}"#;
        let err = parse_gemini_output(raw).unwrap_err();
        assert!(matches!(err, SpError::LlmParseFailed(_)));
    }

    #[test]
    fn test_parse_gemini_output_bad_outer_json() {
        let raw = "this is not json";
        let err = parse_gemini_output(raw).unwrap_err();
        assert!(matches!(err, SpError::LlmParseFailed(_)));
    }

    #[test]
    fn test_extract_stats_none() {
        let stats = extract_stats(&None);
        assert_eq!(stats.input_tokens, 0);
        assert_eq!(stats.output_tokens, 0);
        assert_eq!(stats.total_latency_ms, 0);
        assert_eq!(stats.total_tool_calls, 0);
    }

    // ── Tests for real-world failure modes observed in production ──

    #[test]
    fn test_extract_json_report_multiline_fenced() {
        // Real failure: charset-normalizer — valid JSON in ```json fences with newlines
        let response = "```json\n{\n  \"verdict\": \"approved\",\n  \"risk_score\": 0.0,\n  \"reasoning\": \"The codebase was audited.\",\n  \"files_reviewed\": [\"./setup.py\"],\n  \"findings\": [\n    {\n      \"severity\": \"info\",\n      \"file_path\": \"./setup.py\",\n      \"description\": \"Standard setup.\",\n      \"confidence\": 1.0,\n      \"category\": \"legitimate\"\n    }\n  ]\n}\n```";
        let report = extract_json_report(response).unwrap();
        assert_eq!(report.verdict, "approved");
        assert_eq!(report.findings.len(), 1);
        assert_eq!(
            report.reasoning.as_deref(),
            Some("The codebase was audited.")
        );
    }

    #[test]
    fn test_extract_json_report_no_reasoning_field() {
        // Real failure: diff review response has no 'reasoning' field
        let response = "{\"verdict\": \"approved\", \"risk_score\": 0.0, \"commit_range\": \"abc..def\", \"findings\": []}";
        let report = extract_json_report(response).unwrap();
        assert_eq!(report.verdict, "approved");
        assert!(report.reasoning.is_none());
    }

    #[test]
    fn test_extract_json_report_prose_only() {
        // Real failure: idna — LLM returned prose, no JSON at all
        let response = "The security audit of the `idna` package is complete. No supply chain vulnerabilities or malicious code were identified. The full report is available in `security_audit_report.json`.";
        let err = extract_json_report(response).unwrap_err();
        assert!(matches!(err, SpError::LlmParseFailed(_)));
    }

    #[test]
    fn test_extract_json_report_prose_with_embedded_json() {
        // Possible failure: prose with JSON embedded in the middle
        let response = "Here is the analysis report:\n\n{\"verdict\": \"rejected\", \"risk_score\": 0.95, \"findings\": [{\"severity\": \"critical\", \"file_path\": \"setup.py\", \"description\": \"Exfiltration\"}]}\n\nPlease review the findings above.";
        let report = extract_json_report(response).unwrap();
        assert_eq!(report.verdict, "rejected");
        assert_eq!(report.findings.len(), 1);
    }

    #[test]
    fn test_extract_json_report_with_escaped_strings() {
        // JSON with escaped quotes and special chars inside string values
        let response = "{\"verdict\": \"approved\", \"risk_score\": 0.1, \"reasoning\": \"Found \\\"exec\\\" but it's safe.\", \"findings\": [{\"severity\": \"info\", \"file_path\": \"src/config.py\", \"description\": \"Uses exec(compile(...)) for config loading — legitimate.\"}]}";
        let report = extract_json_report(response).unwrap();
        assert_eq!(report.verdict, "approved");
        assert!(report.reasoning.as_ref().unwrap().contains("exec"));
    }

    #[test]
    fn test_extract_json_report_with_nested_braces_in_strings() {
        // JSON with { and } inside string values that shouldn't confuse brace matching
        let response = "{\"verdict\": \"approved\", \"risk_score\": 0.0, \"findings\": [{\"severity\": \"info\", \"file_path\": \"test.py\", \"description\": \"Uses format string like {name} but not malicious.\"}]}";
        let report = extract_json_report(response).unwrap();
        assert_eq!(report.verdict, "approved");
    }

    #[test]
    fn test_extract_json_report_large_multiline_fenced() {
        // Simulate the real charset-normalizer failure: large fenced JSON with many findings
        let mut findings = String::new();
        for i in 0..15 {
            if i > 0 {
                findings.push_str(",\n    ");
            }
            findings.push_str(&format!(
                "{{\"severity\": \"info\", \"file_path\": \"./src/file{i}.py\", \"line_range\": \"{}\", \"description\": \"Legitimate usage of standard library.\", \"confidence\": 1.0, \"category\": \"legitimate\"}}",
                i * 10
            ));
        }
        let response = format!(
            "```json\n{{\n  \"verdict\": \"approved\",\n  \"risk_score\": 0.0,\n  \"reasoning\": \"Full audit complete.\",\n  \"files_reviewed\": [\"file1.py\", \"file2.py\"],\n  \"grep_hits\": 24,\n  \"findings\": [\n    {findings}\n  ]\n}}\n```"
        );
        let report = extract_json_report(&response).unwrap();
        assert_eq!(report.verdict, "approved");
        assert_eq!(report.findings.len(), 15);
        assert_eq!(report.grep_hits, Some(24));
    }

    #[test]
    fn test_extract_json_object_basic() {
        let s = "some text {\"key\": \"value\"} more text";
        let extracted = extract_json_object(s).unwrap();
        assert_eq!(extracted, "{\"key\": \"value\"}");
    }

    #[test]
    fn test_extract_json_object_nested() {
        let s = "prefix {\"a\": {\"b\": 1}, \"c\": [1,2]} suffix";
        let extracted = extract_json_object(s).unwrap();
        assert_eq!(extracted, "{\"a\": {\"b\": 1}, \"c\": [1,2]}");
    }

    #[test]
    fn test_extract_json_object_with_braces_in_strings() {
        let s = "{\"desc\": \"uses {fmt} style\", \"val\": 1}";
        let extracted = extract_json_object(s).unwrap();
        assert_eq!(extracted, s);
    }

    #[test]
    fn test_extract_json_object_no_json() {
        let s = "no json here at all";
        assert!(extract_json_object(s).is_none());
    }

    #[test]
    fn test_sanitize_json_escapes() {
        // \x is not a valid JSON escape — should become \\x
        let input = r#"{"desc": "byte \x00 found"}"#;
        let sanitized = sanitize_json_escapes(input);
        assert!(serde_json::from_str::<serde_json::Value>(&sanitized).is_ok());
    }

    #[test]
    fn test_sanitize_preserves_valid_escapes() {
        let input = r#"{"desc": "line\nbreak and \"quotes\" and \\path"}"#;
        let sanitized = sanitize_json_escapes(input);
        let parsed: serde_json::Value = serde_json::from_str(&sanitized).unwrap();
        let desc = parsed["desc"].as_str().unwrap();
        assert!(desc.contains('\n'));
        assert!(desc.contains('"'));
        assert!(desc.contains('\\'));
    }

    #[test]
    fn test_extract_json_report_with_invalid_escapes() {
        // Real failure: LLM embeds source code with \x hex escapes in descriptions
        let response = r#"```json
{
  "verdict": "approved",
  "risk_score": 0.0,
  "reasoning": "Clean package.",
  "findings": [
    {
      "severity": "info",
      "file_path": "./constant.py",
      "description": "Legitimate byte literal \x00\xff used for BOM detection.",
      "confidence": 1.0,
      "category": "legitimate"
    }
  ]
}
```"#;
        let report = extract_json_report(response).unwrap();
        assert_eq!(report.verdict, "approved");
        assert_eq!(report.findings.len(), 1);
    }
}
