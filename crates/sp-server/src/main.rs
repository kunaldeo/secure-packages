mod cli;
mod config;
mod jobs;
mod routes;
mod state;

use std::sync::Arc;

use apalis::prelude::*;
use apalis_sql::postgres::PostgresStorage;
use clap::Parser;
use tracing_subscriber::{EnvFilter, fmt};

use crate::cli::{Cli, Commands};
use crate::config::AppConfig;
use crate::jobs::AnalyzeJob;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .json()
        .init();

    let cli = Cli::parse();
    let config = AppConfig::load()?;

    match cli.command {
        Commands::Serve => cmd_serve(config).await,
        Commands::Worker => cmd_worker(config).await,
        Commands::Migrate => cmd_migrate(config).await,
        Commands::Analyze { package, version } => cmd_analyze(config, package, version).await,
    }
}

async fn cmd_serve(config: AppConfig) -> anyhow::Result<()> {
    let pool = sp_db::create_pool(&config.database.url, config.database.max_connections).await?;

    // Set up apalis schema (idempotent)
    PostgresStorage::<()>::setup(&pool).await?;

    if config.database.run_migrations {
        tracing::info!("Running app schema migrations");
        let sql = include_str!("../../../migrations/001_initial.sql");
        // Use IF NOT EXISTS patterns in the SQL to make this idempotent
        sqlx::raw_sql(sql).execute(&pool).await?;
    }
    let storage: PostgresStorage<AnalyzeJob> = PostgresStorage::new(pool.clone());

    let state = AppState {
        db: pool.clone(),
        config: Arc::new(config.clone()),
        job_storage: storage.clone(),
    };

    let app = routes::router(state);

    // Build apalis worker
    let worker_state = AppState {
        db: pool.clone(),
        config: Arc::new(config.clone()),
        job_storage: storage.clone(),
    };

    let worker = WorkerBuilder::new("analysis-worker")
        .concurrency(config.worker.concurrency)
        .data(worker_state)
        .backend(storage)
        .build_fn(jobs::handle_analyze);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!(addr = %addr, "Starting server");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // Run both axum server and apalis monitor concurrently
    let server = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal());

    let monitor = Monitor::new().register(worker).run();

    tokio::select! {
        result = server => { result?; }
        result = monitor => { result?; }
    }

    Ok(())
}

async fn cmd_worker(config: AppConfig) -> anyhow::Result<()> {
    let pool = sp_db::create_pool(&config.database.url, config.database.max_connections).await?;

    PostgresStorage::<()>::setup(&pool).await?;
    let storage: PostgresStorage<AnalyzeJob> = PostgresStorage::new(pool.clone());

    let state = AppState {
        db: pool.clone(),
        config: Arc::new(config.clone()),
        job_storage: storage.clone(),
    };

    let worker = WorkerBuilder::new("analysis-worker")
        .concurrency(config.worker.concurrency)
        .data(state)
        .backend(storage)
        .build_fn(jobs::handle_analyze);

    tracing::info!("Starting worker");
    Monitor::new().register(worker).run().await?;

    Ok(())
}

async fn cmd_migrate(config: AppConfig) -> anyhow::Result<()> {
    let pool = sp_db::create_pool(&config.database.url, config.database.max_connections).await?;

    // Run apalis migrations first (they use timestamp-based versions)
    tracing::info!("Running apalis schema setup");
    PostgresStorage::<()>::setup(&pool).await?;

    // Run our app migrations (they use a separate sqlx migrator instance,
    // so we need to use raw SQL to avoid conflicts with apalis's _sqlx_migrations)
    tracing::info!("Running app schema setup");
    let sql = include_str!("../../../migrations/001_initial.sql");
    sqlx::raw_sql(sql).execute(&pool).await?;

    tracing::info!("All migrations complete");
    Ok(())
}

async fn cmd_analyze(config: AppConfig, package: String, version: String) -> anyhow::Result<()> {
    use sp_analysis::{AnalysisConfig, AnalysisOrchestrator, GeminiRunner};
    use sp_core::{Ecosystem, PackageId, PackageVersion, RegistryClient};
    use sp_registry_pypi::{PyPIRegistryClient, SourceCache};
    use std::path::PathBuf;

    tracing::info!(package = %package, version = %version, "Starting manual analysis");

    let skills_dir = PathBuf::from("skills/pypi");
    let gemini = GeminiRunner::new(
        &config.analysis.gemini_model,
        &skills_dir,
        config.analysis.gemini_timeout_seconds,
        config.analysis.gemini_binary.as_deref(),
    );
    let orchestrator = AnalysisOrchestrator::new(
        gemini,
        AnalysisConfig {
            max_source_size_bytes: config.analysis.max_source_size_bytes,
        },
    );

    let cache = SourceCache::new(&config.cache.source_cache_dir);
    let pypi_client = PyPIRegistryClient::new(
        &config.pypi.upstream_index,
        &config.pypi.upstream_json_api,
        cache,
    );

    let pv = PackageVersion {
        package: PackageId {
            ecosystem: Ecosystem::PyPI,
            name: package.clone(),
        },
        version: version.clone(),
    };

    tracing::info!("Fetching source from PyPI");
    let source = pypi_client.fetch_source(&pv).await?;

    tracing::info!(path = %source.extracted_path.display(), "Running analysis");
    let result = orchestrator.analyze_new(&source.extracted_path).await?;

    // Print results as JSON
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C handler");
    tracing::info!("Shutdown signal received");
}
