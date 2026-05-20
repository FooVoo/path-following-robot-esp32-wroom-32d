//! Log API handlers — `/robots/:id/logs`.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use crate::server::{
    domain::RobotId,
    http::state::AppState,
};

#[derive(Deserialize)]
pub struct LogsQuery {
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 { 100 }

pub async fn robot_logs(
    Path(id): Path<String>,
    Query(params): Query<LogsQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let robot_id = RobotId::new(&id);

    match state.query_logs_for(&robot_id, params.limit, params.offset).await {
        Ok((logs, total)) => Json(serde_json::json!({
            "robot_id": id,
            "total":    total,
            "limit":    params.limit,
            "offset":   params.offset,
            "logs":     logs,
        }))
        .into_response(),

        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": format!("{e:?}")})),
        )
            .into_response(),
    }
}
