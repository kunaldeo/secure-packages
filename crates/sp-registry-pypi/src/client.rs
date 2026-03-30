use std::path::Path;

use async_trait::async_trait;
use scraper::{Html, Selector};
use tracing::{debug, info, warn};

use sp_core::{Ecosystem, PackageVersion, RegistryClient, SourceArchive, SpError};

use crate::normalize::normalize_name;
use crate::source_cache::{SourceCache, TempWorkspace};

/// A parsed link from the PEP 503 simple index HTML.
#[derive(Debug, Clone)]
pub struct SimpleIndexLink {
    pub filename: String,
    pub url: String,
    pub sha256: Option<String>,
    pub requires_python: Option<String>,
    pub yanked: Option<String>,
}

pub struct PyPIRegistryClient {
    http: reqwest::Client,
    simple_index_url: String,
    json_api_url: String,
    cache: SourceCache,
}

impl PyPIRegistryClient {
    pub fn new(
        simple_index_url: impl Into<String>,
        json_api_url: impl Into<String>,
        cache: SourceCache,
    ) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("secure-packages/0.1")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            simple_index_url: simple_index_url.into(),
            json_api_url: json_api_url.into(),
            cache,
        }
    }

    /// Fetch and parse the PEP 503 simple index page for a package.
    async fn fetch_simple_index(&self, package: &str) -> Result<Vec<SimpleIndexLink>, SpError> {
        let normalized = normalize_name(package);
        let url = format!(
            "{}/{}/",
            self.simple_index_url.trim_end_matches('/'),
            normalized
        );

        debug!(url = %url, "Fetching simple index");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| SpError::Http(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SpError::PackageNotFound {
                ecosystem: "pypi".to_string(),
                package: package.to_string(),
            });
        }

        let html = resp
            .text()
            .await
            .map_err(|e| SpError::Http(e.to_string()))?;

        parse_simple_index_html(&html)
    }

    /// Download the raw sdist bytes for a given URL.
    async fn download_sdist(&self, url: &str) -> Result<Vec<u8>, SpError> {
        info!(url = %url, "Downloading sdist");

        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| SpError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(SpError::Http(format!(
                "Failed to download {}: {}",
                url,
                resp.status()
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| SpError::Http(e.to_string()))?;

        Ok(bytes.to_vec())
    }

    /// Find the sdist link for a specific version from the simple index links.
    fn find_sdist_link<'a>(
        links: &'a [SimpleIndexLink],
        version: &str,
    ) -> Option<&'a SimpleIndexLink> {
        links.iter().find(|link| {
            let fname = &link.filename;
            // sdists are .tar.gz or .zip, and contain the version in the filename
            (fname.ends_with(".tar.gz") || fname.ends_with(".zip")) && fname.contains(version)
        })
    }

    /// Extract a .tar.gz archive into a target directory.
    fn extract_targz(archive_path: &Path, target_dir: &Path) -> Result<(), SpError> {
        use std::fs::File;
        use std::io::BufReader;

        let file = File::open(archive_path)?;
        let reader = BufReader::new(file);
        let decoder = flate2::read::GzDecoder::new(reader);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(target_dir)?;

        Ok(())
    }
}

#[async_trait]
impl RegistryClient for PyPIRegistryClient {
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::PyPI
    }

    async fn list_versions(&self, package_name: &str) -> Result<Vec<String>, SpError> {
        let links = self.fetch_simple_index(package_name).await?;

        // Extract version from sdist filenames.
        // Pattern: {name}-{version}.tar.gz
        let normalized = normalize_name(package_name);
        let mut versions = Vec::new();

        for link in &links {
            if !link.filename.ends_with(".tar.gz") {
                continue;
            }
            // Strip the .tar.gz suffix
            let stem = link
                .filename
                .strip_suffix(".tar.gz")
                .unwrap_or(&link.filename);
            // Version comes after the last hyphen that follows the package name
            // e.g. "requests-2.31.0" → "2.31.0"
            let prefix = format!("{}-", normalized);
            if let Some(version) = stem.to_lowercase().strip_prefix(&prefix) {
                versions.push(version.to_string());
            }
        }

        versions.sort_by(|a, b| compare_pep440(a, b));
        versions.dedup();
        Ok(versions)
    }

    async fn fetch_source(&self, pv: &PackageVersion) -> Result<SourceArchive, SpError> {
        let links = self.fetch_simple_index(&pv.package.name).await?;

        let sdist_link =
            Self::find_sdist_link(&links, &pv.version).ok_or_else(|| SpError::NoSourceDist {
                package: pv.package.name.clone(),
                version: pv.version.clone(),
            })?;

        // Check if yanked
        if let Some(reason) = &sdist_link.yanked {
            return Err(SpError::Yanked {
                package: pv.package.name.clone(),
                version: pv.version.clone(),
                reason: reason.clone(),
            });
        }

        let expected_sha256 = sdist_link.sha256.clone().unwrap_or_default();
        let url = sdist_link.url.clone();

        // Use cache
        let archive_path = self
            .cache
            .get_or_fetch(
                "pypi",
                &normalize_name(&pv.package.name),
                &pv.version,
                &expected_sha256,
                || self.download_sdist(&url),
            )
            .await?;

        // Verify hash if we have one
        if !expected_sha256.is_empty() {
            let actual = sha256_file(&archive_path)?;
            if actual != expected_sha256 {
                return Err(SpError::HashMismatch {
                    filename: sdist_link.filename.clone(),
                    expected: expected_sha256,
                    actual,
                });
            }
        }

        // Extract into a TempWorkspace. The workspace is moved into SourceArchive
        // to keep the temp directory alive for the caller to use.
        let workspace = TempWorkspace::new()?;
        Self::extract_targz(&archive_path, workspace.path())?;

        let extracted_path = find_extracted_root(workspace.path())?;

        Ok(SourceArchive {
            package_version: pv.clone(),
            archive_path: archive_path.clone(),
            extracted_path,
            sha256: if expected_sha256.is_empty() {
                sha256_file(&archive_path)?
            } else {
                expected_sha256
            },
            _keep_alive: Some(std::sync::Arc::new(workspace.into_inner())),
        })
    }

    async fn fetch_metadata(&self, pv: &PackageVersion) -> Result<serde_json::Value, SpError> {
        let url = format!(
            "{}/{}/{}/json",
            self.json_api_url.trim_end_matches('/'),
            normalize_name(&pv.package.name),
            pv.version
        );

        debug!(url = %url, "Fetching PyPI JSON metadata");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| SpError::Http(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SpError::VersionNotFound {
                package: pv.package.name.clone(),
                version: pv.version.clone(),
            });
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SpError::Http(e.to_string()))?;

        Ok(json)
    }
}

/// Parse PEP 503 simple index HTML into a list of links.
pub fn parse_simple_index_html(html: &str) -> Result<Vec<SimpleIndexLink>, SpError> {
    let document = Html::parse_document(html);
    let a_selector =
        Selector::parse("a").map_err(|e| SpError::Other(format!("CSS selector error: {e:?}")))?;

    let mut links = Vec::new();

    for element in document.select(&a_selector) {
        let href = match element.value().attr("href") {
            Some(h) => h.to_string(),
            None => continue,
        };

        // Extract filename from href (last path segment, before #fragment)
        let filename = href
            .split('#')
            .next()
            .unwrap_or(&href)
            .rsplit('/')
            .next()
            .unwrap_or(&href)
            .to_string();

        // Extract sha256 from fragment: #sha256=abc123
        let sha256 = href
            .split_once("#sha256=")
            .map(|(_, hash)| hash.to_string());

        let requires_python = element
            .value()
            .attr("data-requires-python")
            .map(|s| s.to_string());

        let yanked = element.value().attr("data-yanked").map(|s| {
            if s.is_empty() {
                "yanked".to_string()
            } else {
                s.to_string()
            }
        });

        links.push(SimpleIndexLink {
            filename,
            url: href,
            sha256,
            requires_python,
            yanked,
        });
    }

    Ok(links)
}

/// Compare two PEP 440 version strings. Falls back to string comparison if parsing fails.
pub fn compare_pep440(a: &str, b: &str) -> std::cmp::Ordering {
    use pep440_rs::Version;

    let va = a.parse::<Version>();
    let vb = b.parse::<Version>();

    match (va, vb) {
        (Ok(va), Ok(vb)) => va.cmp(&vb),
        _ => a.cmp(b),
    }
}

/// Compute SHA-256 hex digest of a file.
fn sha256_file(path: &Path) -> Result<String, SpError> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Find the root directory inside an extracted archive.
/// Most sdists extract to a single directory like "requests-2.31.0/".
fn find_extracted_root(workspace: &Path) -> Result<std::path::PathBuf, SpError> {
    let mut entries: Vec<_> = std::fs::read_dir(workspace)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();

    // Skip .gemini directory if we created one
    entries.retain(|e| {
        e.file_name()
            .to_str()
            .map(|n| n != ".gemini")
            .unwrap_or(true)
    });

    match entries.len() {
        1 => Ok(entries.remove(0).path()),
        0 => Ok(workspace.to_path_buf()), // No subdirectory, source is at root
        _ => {
            warn!(
                "Multiple directories in extracted archive, using first: {:?}",
                entries[0].path()
            );
            Ok(entries.remove(0).path())
        }
    }
}

// SHA-256 hashing using the sha2 crate via a simple import
use sha2::{Digest, Sha256};

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SIMPLE_HTML: &str = r#"
<!DOCTYPE html>
<html>
<body>
<a href="https://files.example.com/packages/requests-2.31.0.tar.gz#sha256=abc123def456">requests-2.31.0.tar.gz</a>
<a href="https://files.example.com/packages/requests-2.31.0-py3-none-any.whl#sha256=789xyz">requests-2.31.0-py3-none-any.whl</a>
<a href="https://files.example.com/packages/requests-2.30.0.tar.gz#sha256=old123" data-requires-python="&gt;=3.7">requests-2.30.0.tar.gz</a>
<a href="https://files.example.com/packages/requests-2.29.0.tar.gz#sha256=yanked1" data-yanked="security issue">requests-2.29.0.tar.gz</a>
</body>
</html>
"#;

    #[test]
    fn test_parse_simple_index_html() {
        let links = parse_simple_index_html(SAMPLE_SIMPLE_HTML).unwrap();
        assert_eq!(links.len(), 4);

        // First link: sdist with hash
        assert_eq!(links[0].filename, "requests-2.31.0.tar.gz");
        assert_eq!(links[0].sha256.as_deref(), Some("abc123def456"));
        assert!(links[0].yanked.is_none());

        // Second link: wheel
        assert_eq!(links[1].filename, "requests-2.31.0-py3-none-any.whl");

        // Third link: older sdist with requires-python
        assert_eq!(links[2].requires_python.as_deref(), Some(">=3.7"));

        // Fourth link: yanked
        assert_eq!(links[3].yanked.as_deref(), Some("security issue"));
    }

    #[test]
    fn test_find_sdist_link() {
        let links = parse_simple_index_html(SAMPLE_SIMPLE_HTML).unwrap();

        let found = PyPIRegistryClient::find_sdist_link(&links, "2.31.0");
        assert!(found.is_some());
        assert_eq!(found.unwrap().filename, "requests-2.31.0.tar.gz");

        let found = PyPIRegistryClient::find_sdist_link(&links, "9.99.99");
        assert!(found.is_none());
    }

    #[test]
    fn test_yanked_detection() {
        let links = parse_simple_index_html(SAMPLE_SIMPLE_HTML).unwrap();

        let yanked = PyPIRegistryClient::find_sdist_link(&links, "2.29.0").unwrap();
        assert_eq!(yanked.yanked.as_deref(), Some("security issue"));

        let not_yanked = PyPIRegistryClient::find_sdist_link(&links, "2.31.0").unwrap();
        assert!(not_yanked.yanked.is_none());
    }

    #[test]
    fn test_pep440_ordering() {
        use std::cmp::Ordering;

        assert_eq!(compare_pep440("1.0.0", "2.0.0"), Ordering::Less);
        assert_eq!(compare_pep440("2.0.0", "2.0.0"), Ordering::Equal);
        assert_eq!(compare_pep440("2.31.0", "2.4.0"), Ordering::Greater);

        // Pre-releases
        assert_eq!(compare_pep440("1.0a1", "1.0b1"), Ordering::Less);
        assert_eq!(compare_pep440("1.0b1", "1.0rc1"), Ordering::Less);
        assert_eq!(compare_pep440("1.0rc1", "1.0"), Ordering::Less);

        // Post-releases
        assert_eq!(compare_pep440("1.0", "1.0.post1"), Ordering::Less);
    }

    #[test]
    fn test_normalize_in_version_extraction() {
        // Verify that the list_versions logic would correctly parse versions
        let html = r#"
<html><body>
<a href="my-package-1.0.tar.gz#sha256=a">my-package-1.0.tar.gz</a>
<a href="my-package-2.0.tar.gz#sha256=b">my-package-2.0.tar.gz</a>
<a href="my-package-2.0-py3-none-any.whl#sha256=c">my-package-2.0-py3-none-any.whl</a>
</body></html>
"#;
        let links = parse_simple_index_html(html).unwrap();
        let sdists: Vec<_> = links
            .iter()
            .filter(|l| l.filename.ends_with(".tar.gz"))
            .collect();
        assert_eq!(sdists.len(), 2);
    }

    #[test]
    fn test_sha256_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello world").unwrap();
        let hash = sha256_file(&path).unwrap();
        // Known SHA-256 of "hello world"
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_find_extracted_root_single_dir() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("requests-2.31.0");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::write(subdir.join("setup.py"), "").unwrap();

        let root = find_extracted_root(dir.path()).unwrap();
        assert_eq!(root, subdir);
    }

    #[test]
    fn test_find_extracted_root_no_subdir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("setup.py"), "").unwrap();

        let root = find_extracted_root(dir.path()).unwrap();
        assert_eq!(root, dir.path());
    }
}
