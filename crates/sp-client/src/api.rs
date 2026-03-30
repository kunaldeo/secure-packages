use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::resolver::ResolvedPackage;

/// Status of a single package from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageStatus {
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

/// Full analysis details from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisDetails {
    pub version_id: Uuid,
    pub package: String,
    pub version: String,
    pub ecosystem: String,
    pub status: String,
    pub analysis: Option<AnalysisInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisInfo {
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

#[derive(Debug, Serialize)]
struct CheckRequest {
    name: String,
    version: String,
    ecosystem: String,
}

pub struct SpClient {
    http: reqwest::Client,
    server_url: String,
}

impl SpClient {
    pub fn new(server_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            server_url: server_url.into().trim_end_matches('/').to_string(),
        }
    }

    /// Check the status of multiple packages. Triggers analysis for unknown ones.
    pub async fn check_packages(
        &self,
        packages: &[ResolvedPackage],
    ) -> Result<Vec<PackageStatus>, ClientError> {
        let body: Vec<CheckRequest> = packages
            .iter()
            .map(|p| CheckRequest {
                name: p.name.clone(),
                version: p.version.clone(),
                ecosystem: "pypi".to_string(),
            })
            .collect();

        let resp = self
            .http
            .post(format!("{}/api/v1/packages/check", self.server_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| ClientError::Connection(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ClientError::Server(format!(
                "Server returned {}",
                resp.status()
            )));
        }

        let statuses: Vec<PackageStatus> = resp
            .json()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))?;

        Ok(statuses)
    }

    /// Get full analysis details for a specific package version.
    pub async fn get_analysis_details(
        &self,
        name: &str,
        version: &str,
    ) -> Result<AnalysisDetails, ClientError> {
        let resp = self
            .http
            .get(format!(
                "{}/api/v1/packages/{}/versions/{}",
                self.server_url, name, version
            ))
            .send()
            .await
            .map_err(|e| ClientError::Connection(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ClientError::NotFound(format!("{name}=={version}")));
        }

        if !resp.status().is_success() {
            return Err(ClientError::Server(format!(
                "Server returned {}",
                resp.status()
            )));
        }

        let details: AnalysisDetails = resp
            .json()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))?;

        Ok(details)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Server error: {0}")]
    Server(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Parse error: {0}")]
    Parse(String),
}
