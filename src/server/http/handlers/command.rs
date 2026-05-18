//! Command handlers — `/command/...`.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use crate::server::{
    application::command_service::CommandError,
    domain::{RobotCommand, RobotId},
    http::state::AppState,
};

#[derive(Deserialize)]
pub struct ThrottleRequest {
    left:  i8,
    right: i8,
}

fn command_error_response(e: CommandError) -> impl IntoResponse {
    let (status, msg) = match &e {
        CommandError::NoRobot       => (StatusCode::SERVICE_UNAVAILABLE, format!("{e}")),
        CommandError::Gateway(_)    => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")),
        CommandError::Repo(_)       => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")),
    };
    (status, Json(serde_json::json!({"error": msg}))).into_response()
}

// ── Generic (most-recently-seen robot) ────────────────────────────────────────

pub async fn cmd_button(State(state): State<AppState>) -> impl IntoResponse {
    match state.cmd_svc.send_to_recent(RobotCommand::Button).await {
        Ok(())  => Json(serde_json::json!({"status": "sent"})).into_response(),
        Err(e)  => command_error_response(e).into_response(),
    }
}

pub async fn cmd_throttle(
    State(state): State<AppState>,
    Json(req): Json<ThrottleRequest>,
) -> impl IntoResponse {
    let cmd = RobotCommand::Throttle { left: req.left, right: req.right };
    match state.cmd_svc.send_to_recent(cmd).await {
        Ok(())  => Json(serde_json::json!({"status": "sent"})).into_response(),
        Err(e)  => command_error_response(e).into_response(),
    }
}

// ── Per-robot ─────────────────────────────────────────────────────────────────

pub async fn cmd_button_for(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let robot_id = RobotId::new(&id);
    match state.cmd_svc.send_to(&robot_id, RobotCommand::Button).await {
        Ok(())  => Json(serde_json::json!({"status": "sent", "to": id})).into_response(),
        Err(e)  => command_error_response(e).into_response(),
    }
}

pub async fn cmd_throttle_for(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<ThrottleRequest>,
) -> impl IntoResponse {
    let robot_id = RobotId::new(&id);
    let cmd = RobotCommand::Throttle { left: req.left, right: req.right };
    match state.cmd_svc.send_to(&robot_id, cmd).await {
        Ok(())  => Json(serde_json::json!({"status": "sent", "to": id})).into_response(),
        Err(e)  => command_error_response(e).into_response(),
    }
}
