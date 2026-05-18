//! Fleet JSON API handlers — `/robots`, `/health`, `/telemetry`.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
};
use serde::Serialize;

use crate::server::http::state::AppState;

#[derive(Serialize)]
pub(crate) struct HealthResponse {
    status: &'static str,
    robots_online: usize,
    robot_ips: Vec<String>,
}

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let robots = state.query_svc.list_robots().await.unwrap_or_default();
    let mut robot_ips: Vec<String> = robots.iter().map(|s| s.id.as_str().to_owned()).collect();
    robot_ips.sort();
    Json(HealthResponse {
        status: "ok",
        robots_online: robot_ips.len(),
        robot_ips,
    })
}

pub async fn list_robots(State(state): State<AppState>) -> impl IntoResponse {
    match state.query_svc.list_robots().await {
        Ok(robots) => Json(serde_json::to_value(&robots).unwrap_or_default()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e:?}")})),
        )
            .into_response(),
    }
}

pub async fn get_telemetry(State(state): State<AppState>) -> impl IntoResponse {
    match state.query_svc.most_recent_robot().await {
        Ok(Some(snap)) => {
            (StatusCode::OK, Json(serde_json::to_value(&snap.latest).unwrap())).into_response()
        }
        Ok(None) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no telemetry received yet"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e:?}")})),
        )
            .into_response(),
    }
}
