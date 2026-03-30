mod api;
mod health;

use axum::Router;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(health::routes())
        .nest("/api/v1", api::routes())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
