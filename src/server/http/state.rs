//! HTTP layer — shared application state for axum handlers.

use std::sync::Arc;

use crate::server::{
    application::{CommandService, FleetQueryService, IngestTelemetryService},
    domain::{LogEntry, RobotId},
    infrastructure::{
        BroadcastEventPublisher, InMemoryFleetRepository, PostgresFleetRepository,
        UdpCommandGateway,
    },
    ports::{FleetRepository, fleet_repository::RepoResult},
};

/// Shared state injected into every axum handler via `State<AppState>`.
///
/// All fields are `Arc`-wrapped, so `AppState::clone()` is cheap.
#[derive(Clone)]
pub struct AppState {
    /// Read-side: list robots, get snapshots, paginate logs.
    pub query_svc: Arc<FleetQueryService<InMemoryFleetRepository>>,
    /// Write-side: send commands to robots.
    pub cmd_svc: Arc<CommandService<InMemoryFleetRepository, UdpCommandGateway>>,
    /// SSE: allows handlers to create new event subscriptions.
    pub events: Arc<BroadcastEventPublisher>,
    /// Optional Postgres repo for log queries (has rich history).
    /// If `None`, log queries fall back to the in-memory store.
    pub pg_repo: Option<Arc<PostgresFleetRepository>>,
    /// Direct access to in-memory repo for log queries when Postgres is absent.
    pub mem_repo: Arc<InMemoryFleetRepository>,
}

impl AppState {
    /// Return a page of telemetry log entries for `robot_id`.
    ///
    /// Prefers Postgres when available (richer history, survives restarts);
    /// falls back to the in-memory store otherwise.
    pub async fn query_logs_for(
        &self,
        robot_id: &RobotId,
        limit: i64,
        offset: i64,
    ) -> RepoResult<(Vec<LogEntry>, i64)> {
        if let Some(ref pg) = self.pg_repo {
            let (logs, total) = tokio::join!(
                pg.query_logs(robot_id, limit, offset),
                pg.count_logs(robot_id),
            );
            Ok((logs?, total?))
        } else {
            self.query_svc.query_logs(robot_id, limit, offset).await
        }
    }
}
