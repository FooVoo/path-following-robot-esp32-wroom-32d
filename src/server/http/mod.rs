//! HTTP layer — axum router and shared application state.

pub mod handlers;
pub mod state;

use axum::{
    Router,
    routing::{get, post},
};

use state::AppState;

/// Build the axum `Router` with all routes and shared state injected.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // UI pages
        .route("/",                           get(handlers::ui::fleet_ui))
        .route("/robots/{id}",                get(handlers::ui::robot_ui))
        // SSE
        .route("/events",                     get(handlers::sse::sse_handler))
        // JSON API — fleet
        .route("/health",                     get(handlers::fleet::health))
        .route("/robots",                     get(handlers::fleet::list_robots))
        .route("/telemetry",                  get(handlers::fleet::get_telemetry))
        // JSON API — logs
        .route("/robots/{id}/logs",           get(handlers::logs::robot_logs))
        // Commands — generic (targets most-recently-seen robot)
        .route("/command/button",             post(handlers::command::cmd_button))
        .route("/command/throttle",           post(handlers::command::cmd_throttle))
        // Commands — per-robot
        .route("/command/{id}/button",        post(handlers::command::cmd_button_for))
        .route("/command/{id}/throttle",      post(handlers::command::cmd_throttle_for))
        .with_state(state)
}
