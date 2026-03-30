use async_trait::async_trait;
use serde_json::Value;

use crate::error::SpError;
use crate::types::{Ecosystem, PackageVersion, SourceArchive};

/// Fetches package metadata and source from an upstream registry.
///
/// Each ecosystem (PyPI, npm, etc.) implements this trait.
#[async_trait]
pub trait RegistryClient: Send + Sync {
    /// Which ecosystem this client serves.
    fn ecosystem(&self) -> Ecosystem;

    /// List all available versions for a package.
    async fn list_versions(&self, package_name: &str) -> Result<Vec<String>, SpError>;

    /// Download and extract source code for a specific version.
    async fn fetch_source(&self, pv: &PackageVersion) -> Result<SourceArchive, SpError>;

    /// Fetch upstream metadata (JSON) for a specific version.
    async fn fetch_metadata(&self, pv: &PackageVersion) -> Result<Value, SpError>;
}
