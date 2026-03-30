use std::path::Path;
use std::process::Stdio;

use serde::Deserialize;
use tracing::{debug, info};

/// A resolved package with an exact version.
#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy)]
pub enum Resolver {
    Uv,
    Pip,
}

impl std::fmt::Display for Resolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uv => write!(f, "uv"),
            Self::Pip => write!(f, "pip"),
        }
    }
}

/// Resolve all dependencies from a requirements file to exact versions
/// using pip or uv's --dry-run --report mode.
pub async fn resolve(
    requirements_path: &Path,
    resolver_override: Option<Resolver>,
) -> Result<Vec<ResolvedPackage>, ResolverError> {
    let resolver = match resolver_override {
        Some(r) => r,
        None => detect_resolver(),
    };

    info!(resolver = %resolver, requirements = %requirements_path.display(), "Resolving dependencies");

    match resolver {
        Resolver::Uv => resolve_with_uv(requirements_path).await,
        Resolver::Pip => resolve_with_pip(requirements_path).await,
    }
}

async fn resolve_with_pip(requirements_path: &Path) -> Result<Vec<ResolvedPackage>, ResolverError> {
    let report_file =
        tempfile::NamedTempFile::new().map_err(|e| ResolverError::Io(e.to_string()))?;
    let report_path = report_file.path().to_path_buf();

    let output = tokio::process::Command::new("pip")
        .args(["install", "--dry-run", "--report"])
        .arg(&report_path)
        .arg("-r")
        .arg(requirements_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| ResolverError::NotFound(format!("pip not found: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ResolverError::ResolutionFailed(format!(
            "pip resolution failed (exit {}): {stderr}",
            output.status.code().unwrap_or(-1)
        )));
    }

    let report_json = std::fs::read_to_string(&report_path)
        .map_err(|e| ResolverError::Io(format!("Failed to read report: {e}")))?;

    parse_pip_report(&report_json)
}

async fn resolve_with_uv(requirements_path: &Path) -> Result<Vec<ResolvedPackage>, ResolverError> {
    // uv pip compile reads requirements.txt and outputs pinned versions to stdout
    let output = tokio::process::Command::new("uv")
        .args(["pip", "compile"])
        .arg(requirements_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| ResolverError::NotFound(format!("uv not found: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ResolverError::ResolutionFailed(format!(
            "uv resolution failed (exit {}): {stderr}",
            output.status.code().unwrap_or(-1)
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_uv_compile_output(&stdout)
}

/// Detect whether uv or pip is available on PATH. Prefers uv.
fn detect_resolver() -> Resolver {
    if which("uv") {
        debug!("Auto-detected uv");
        Resolver::Uv
    } else {
        debug!("uv not found, using pip");
        Resolver::Pip
    }
}

fn which(name: &str) -> bool {
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        if dir.join(name).is_file() {
            return true;
        }
    }
    false
}

/// Parse uv pip compile output (pinned requirements format) into resolved packages.
/// Output looks like: "package==1.0.0\n# via ...\nother==2.0\n"
pub fn parse_uv_compile_output(output: &str) -> Result<Vec<ResolvedPackage>, ResolverError> {
    let mut packages = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        // Skip comments, empty lines, and annotation lines
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        // Lines look like "requests==2.31.0" or "urllib3==2.1.0 ; python_version >= '3.7'"
        let spec = line.split(';').next().unwrap_or(line).trim();
        if let Some((name, version)) = spec.split_once("==") {
            packages.push(ResolvedPackage {
                name: sp_registry_pypi::normalize_name(name.trim()),
                version: version.trim().to_string(),
            });
        }
    }

    info!(count = packages.len(), "Resolved packages (uv)");
    Ok(packages)
}

/// Parse the pip --report JSON output into resolved packages.
pub fn parse_pip_report(json: &str) -> Result<Vec<ResolvedPackage>, ResolverError> {
    let report: PipReport =
        serde_json::from_str(json).map_err(|e| ResolverError::ParseFailed(e.to_string()))?;

    let packages: Vec<ResolvedPackage> = report
        .install
        .into_iter()
        .map(|item| ResolvedPackage {
            name: sp_registry_pypi::normalize_name(&item.metadata.name),
            version: item.metadata.version,
        })
        .collect();

    info!(count = packages.len(), "Resolved packages");
    Ok(packages)
}

// ── pip/uv report JSON schema ──

#[derive(Debug, Deserialize)]
struct PipReport {
    #[serde(default)]
    install: Vec<InstallItem>,
}

#[derive(Debug, Deserialize)]
struct InstallItem {
    metadata: PackageMetadata,
}

#[derive(Debug, Deserialize)]
struct PackageMetadata {
    name: String,
    version: String,
}

// ── Errors ──

#[derive(Debug, thiserror::Error)]
pub enum ResolverError {
    #[error("Resolver not found: {0}")]
    NotFound(String),

    #[error("Resolution failed: {0}")]
    ResolutionFailed(String),

    #[error("Failed to parse report: {0}")]
    ParseFailed(String),

    #[error("IO error: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_REPORT: &str = r#"{
        "version": "1",
        "pip_version": "24.0",
        "install": [
            {
                "metadata": {
                    "name": "requests",
                    "version": "2.31.0"
                },
                "download_info": {}
            },
            {
                "metadata": {
                    "name": "urllib3",
                    "version": "2.1.0"
                },
                "download_info": {}
            },
            {
                "metadata": {
                    "name": "certifi",
                    "version": "2024.2.2"
                },
                "download_info": {}
            },
            {
                "metadata": {
                    "name": "charset-normalizer",
                    "version": "3.3.2"
                },
                "download_info": {}
            },
            {
                "metadata": {
                    "name": "idna",
                    "version": "3.6"
                },
                "download_info": {}
            }
        ]
    }"#;

    #[test]
    fn test_parse_report() {
        let packages = parse_pip_report(SAMPLE_REPORT).unwrap();
        assert_eq!(packages.len(), 5);
        assert_eq!(packages[0].name, "requests");
        assert_eq!(packages[0].version, "2.31.0");
        assert_eq!(packages[1].name, "urllib3");
        assert_eq!(packages[4].name, "idna");
    }

    #[test]
    fn test_parse_report_empty() {
        let json = r#"{"version": "1", "install": []}"#;
        let packages = parse_pip_report(json).unwrap();
        assert_eq!(packages.len(), 0);
    }

    #[test]
    fn test_parse_report_normalizes_names() {
        let json = r#"{
            "version": "1",
            "install": [
                {"metadata": {"name": "My_Cool.Package", "version": "1.0.0"}, "download_info": {}}
            ]
        }"#;
        let packages = parse_pip_report(json).unwrap();
        assert_eq!(packages[0].name, "my-cool-package");
    }

    #[test]
    fn test_parse_report_bad_json() {
        let result = parse_pip_report("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_uv_compile_output() {
        let output = r#"# This file was autogenerated by uv via the following command:
#    uv pip compile requirements.txt
certifi==2024.2.2
charset-normalizer==3.3.2
    # via requests
idna==3.6
    # via requests
requests==2.31.0
urllib3==2.1.0
    # via requests
"#;
        let packages = parse_uv_compile_output(output).unwrap();
        assert_eq!(packages.len(), 5);
        assert_eq!(packages[0].name, "certifi");
        assert_eq!(packages[0].version, "2024.2.2");
        assert_eq!(packages[3].name, "requests");
        assert_eq!(packages[3].version, "2.31.0");
    }

    #[test]
    fn test_parse_uv_compile_empty() {
        let output = "# empty\n";
        let packages = parse_uv_compile_output(output).unwrap();
        assert_eq!(packages.len(), 0);
    }

    #[test]
    fn test_parse_uv_compile_with_markers() {
        let output = "requests==2.31.0 ; python_version >= '3.7'\n";
        let packages = parse_uv_compile_output(output).unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "requests");
        assert_eq!(packages[0].version, "2.31.0");
    }
}
