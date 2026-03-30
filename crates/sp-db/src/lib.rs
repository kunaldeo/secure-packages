pub mod models;
pub mod repos;

pub use models::*;
pub use repos::*;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// Migrator for use in tests and startup.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Create a connection pool from a database URL.
pub async fn create_pool(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
}

/// Run migrations from the migrations directory.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}
