use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub analysis: AnalysisConfig,
    pub worker: WorkerConfig,
    pub pypi: PyPIConfig,
    pub cache: CacheConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub admin_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub run_migrations: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisConfig {
    pub gemini_model: String,
    pub gemini_timeout_seconds: u64,
    pub gemini_binary: Option<String>,
    pub max_source_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    pub concurrency: usize,
    pub job_timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PyPIConfig {
    pub upstream_index: String,
    pub upstream_json_api: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub source_cache_dir: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
                admin_token: String::new(),
            },
            database: DatabaseConfig {
                url: "postgres://secure:secure@localhost:5432/secure_packages".to_string(),
                max_connections: 10,
                run_migrations: true,
            },
            analysis: AnalysisConfig {
                gemini_model: "gemini-3.1-flash-lite-preview".to_string(),
                gemini_timeout_seconds: 300,
                gemini_binary: None,
                max_source_size_bytes: 50 * 1024 * 1024,
            },
            worker: WorkerConfig {
                concurrency: 4,
                job_timeout_seconds: 600,
            },
            pypi: PyPIConfig {
                upstream_index: "https://pypi.org/simple/".to_string(),
                upstream_json_api: "https://pypi.org/pypi/".to_string(),
            },
            cache: CacheConfig {
                source_cache_dir: "./data/cache".to_string(),
            },
        }
    }
}

impl AppConfig {
    #[allow(clippy::result_large_err)]
    pub fn load() -> Result<Self, figment::Error> {
        Figment::new()
            .merge(Serialized::defaults(AppConfig::default()))
            .merge(Toml::file("config/default.toml"))
            .merge(Toml::file("config/development.toml"))
            .merge(Env::prefixed("SP_").split("__"))
            .extract()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.server.port, 8080);
        assert_eq!(
            config.analysis.gemini_model,
            "gemini-3.1-flash-lite-preview"
        );
        assert_eq!(config.worker.concurrency, 4);
        assert_eq!(config.pypi.upstream_index, "https://pypi.org/simple/");
    }

    #[test]
    fn test_env_override() {
        // SAFETY: test-only, no concurrent thread reads these vars
        unsafe {
            std::env::set_var("SP_SERVER__PORT", "9090");
            std::env::set_var("SP_ANALYSIS__GEMINI_MODEL", "gemini-2.5-flash");
        }

        let config = Figment::new()
            .merge(Serialized::defaults(AppConfig::default()))
            .merge(Env::prefixed("SP_").split("__"))
            .extract::<AppConfig>()
            .unwrap();

        assert_eq!(config.server.port, 9090);
        assert_eq!(config.analysis.gemini_model, "gemini-2.5-flash");

        unsafe {
            std::env::remove_var("SP_SERVER__PORT");
            std::env::remove_var("SP_ANALYSIS__GEMINI_MODEL");
        }
    }
}
