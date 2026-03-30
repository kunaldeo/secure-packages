use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpError {
    #[error("Package not found: {package} in {ecosystem}")]
    PackageNotFound { ecosystem: String, package: String },

    #[error("Version not found: {package}=={version}")]
    VersionNotFound { package: String, version: String },

    #[error("No source distribution available for {package}=={version}")]
    NoSourceDist { package: String, version: String },

    #[error("SHA256 mismatch for {filename}: expected {expected}, got {actual}")]
    HashMismatch {
        filename: String,
        expected: String,
        actual: String,
    },

    #[error("Package yanked by upstream: {package}=={version}: {reason}")]
    Yanked {
        package: String,
        version: String,
        reason: String,
    },

    #[error("Source too large: {size_bytes} bytes exceeds limit of {limit_bytes} bytes")]
    SourceTooLarge { size_bytes: u64, limit_bytes: u64 },

    #[error("Analysis failed: {0}")]
    AnalysisFailed(String),

    #[error("LLM error: {0}")]
    LlmError(String),

    #[error("LLM response could not be parsed: {0}")]
    LlmParseFailed(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}
