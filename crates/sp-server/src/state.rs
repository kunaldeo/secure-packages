use std::sync::Arc;

use apalis_sql::postgres::PostgresStorage;
use sqlx::PgPool;

use crate::config::AppConfig;
use crate::jobs::AnalyzeJob;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<AppConfig>,
    pub job_storage: PostgresStorage<AnalyzeJob>,
}
