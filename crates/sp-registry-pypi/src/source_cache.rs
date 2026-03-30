use std::path::{Path, PathBuf};

use tracing::{debug, info};

/// Caches downloaded source archives on the filesystem to avoid re-downloading
/// the same sdist on retries or when a diff scan needs both old and new source.
///
/// Key layout: `{cache_dir}/{ecosystem}/{package}/{version}/{sha256}.tar.gz`
/// The cache stores the original archive. Extraction happens into a TempWorkspace.
pub struct SourceCache {
    cache_dir: PathBuf,
}

impl SourceCache {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            cache_dir: cache_dir.into(),
        }
    }

    /// Returns the cached archive path if it exists, otherwise calls `fetch_fn`
    /// to download it and stores the result in the cache.
    pub async fn get_or_fetch<F, Fut>(
        &self,
        ecosystem: &str,
        package: &str,
        version: &str,
        sha256: &str,
        fetch_fn: F,
    ) -> Result<PathBuf, sp_core::SpError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<u8>, sp_core::SpError>>,
    {
        let cache_path = self.archive_path(ecosystem, package, version, sha256);

        if cache_path.exists() {
            debug!(
                path = %cache_path.display(),
                "Source cache hit"
            );
            return Ok(cache_path);
        }

        info!(
            ecosystem,
            package, version, "Source cache miss, downloading"
        );

        let data = fetch_fn().await?;

        if let Some(parent) = cache_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&cache_path, &data)?;
        debug!(
            path = %cache_path.display(),
            bytes = data.len(),
            "Cached source archive"
        );

        Ok(cache_path)
    }

    /// Check if an archive is already cached.
    pub fn contains(&self, ecosystem: &str, package: &str, version: &str, sha256: &str) -> bool {
        self.archive_path(ecosystem, package, version, sha256)
            .exists()
    }

    fn archive_path(&self, ecosystem: &str, package: &str, version: &str, sha256: &str) -> PathBuf {
        self.cache_dir
            .join(ecosystem)
            .join(package)
            .join(version)
            .join(format!("{sha256}.tar.gz"))
    }
}

/// A temporary workspace for a single analysis job. Wraps `tempfile::TempDir`
/// so the extracted source is cleaned up on Drop — even on panic.
pub struct TempWorkspace {
    inner: tempfile::TempDir,
}

impl TempWorkspace {
    pub fn new() -> Result<Self, std::io::Error> {
        let inner = tempfile::TempDir::with_prefix("sp-workspace-")?;
        debug!(path = %inner.path().display(), "Created temp workspace");
        Ok(Self { inner })
    }

    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    /// Consume the workspace and return the inner TempDir, transferring
    /// ownership (and cleanup responsibility) to the caller.
    pub fn into_inner(self) -> tempfile::TempDir {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_miss_then_hit() {
        let cache_dir = tempfile::tempdir().unwrap();
        let cache = SourceCache::new(cache_dir.path());

        let mut call_count = 0u32;

        // First call: cache miss, fetch_fn runs
        let path = cache
            .get_or_fetch("pypi", "requests", "2.31.0", "abc123", || {
                call_count += 1;
                async { Ok(b"fake tarball data".to_vec()) }
            })
            .await
            .unwrap();

        assert!(path.exists());
        assert_eq!(call_count, 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fake tarball data");

        // Second call: cache hit, fetch_fn should NOT be called
        // (We verify by checking the file still exists and content matches)
        assert!(cache.contains("pypi", "requests", "2.31.0", "abc123"));

        let path2 = cache
            .get_or_fetch("pypi", "requests", "2.31.0", "abc123", || async {
                panic!("should not be called on cache hit");
            })
            .await
            .unwrap();

        assert_eq!(path, path2);
    }

    #[tokio::test]
    async fn test_cache_different_versions() {
        let cache_dir = tempfile::tempdir().unwrap();
        let cache = SourceCache::new(cache_dir.path());

        let p1 = cache
            .get_or_fetch("pypi", "flask", "3.0.0", "aaa", || async {
                Ok(b"v3.0.0".to_vec())
            })
            .await
            .unwrap();

        let p2 = cache
            .get_or_fetch("pypi", "flask", "3.1.0", "bbb", || async {
                Ok(b"v3.1.0".to_vec())
            })
            .await
            .unwrap();

        assert_ne!(p1, p2);
        assert_eq!(std::fs::read_to_string(&p1).unwrap(), "v3.0.0");
        assert_eq!(std::fs::read_to_string(&p2).unwrap(), "v3.1.0");
    }

    #[test]
    fn test_temp_workspace_cleanup() {
        let path;
        {
            let ws = TempWorkspace::new().unwrap();
            path = ws.path().to_path_buf();
            std::fs::write(ws.path().join("test.txt"), "data").unwrap();
            assert!(path.exists());
        }
        // After drop, the directory should be gone
        assert!(!path.exists());
    }

    #[test]
    fn test_temp_workspace_cleanup_with_nested_files() {
        let path;
        {
            let ws = TempWorkspace::new().unwrap();
            path = ws.path().to_path_buf();
            let sub = ws.path().join("src").join("pkg");
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(sub.join("main.py"), "print('hello')").unwrap();
            assert!(sub.join("main.py").exists());
        }
        assert!(!path.exists());
    }
}
