//! HTTP layer — shared application state for axum handlers.

use std::sync::Arc;

use crate::server::{
    application::{CommandService, FleetQueryService, IngestTelemetryService},
    infrastructure::{
        BroadcastEventPublisher, InMemoryFleetRepository, PostgresFleetRepository,
        UdpCommandGateway,
    },
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
