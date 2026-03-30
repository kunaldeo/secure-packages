use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};

use crate::state::AppState;

async fn health_check(State(state): State<AppState>) -> Json<Value> {
    let db_ok = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();

    Json(json!({
        "status": if db_ok { "ok" } else { "degraded" },
        "database": if db_ok { "connected" } else { "error" },
    }))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/health", get(health_check))
}
