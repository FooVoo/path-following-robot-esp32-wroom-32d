//! `FleetRepository` — persistence port for robot snapshots and telemetry logs.

use async_trait::async_trait;

use crate::server::domain::{LogEntry, RobotId, RobotSnapshot, TelemetryFrame};

pub type RepoResult<T> = Result<T, RepositoryError>;

#[derive(Debug)]
pub enum RepositoryError {
    NotFound,
    Database(String),
}

impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound      => write!(f, "not found"),
            Self::Database(msg) => write!(f, "database error: {msg}"),
        }
    }
}

/// Persistence interface for the fleet state.
///
/// Implementations: [`PostgresFleetRepository`] (production) and
/// [`InMemoryFleetRepository`] (tests).
///
/// [`PostgresFleetRepository`]: crate::server::infrastructure::postgres_repository::PostgresFleetRepository
/// [`InMemoryFleetRepository`]: crate::server::infrastructure::in_memory_repository::InMemoryFleetRepository
#[async_trait]
pub trait FleetRepository: Send + Sync {
    /// Upsert or insert a robot snapshot.
    async fn save_snapshot(&self, snapshot: &RobotSnapshot) -> RepoResult<()>;

    /// Retrieve a robot snapshot by ID.
    async fn get_snapshot(&self, id: &RobotId) -> RepoResult<Option<RobotSnapshot>>;

    /// List all known robot snapshots, sorted by `last_seen` descending.
    async fn list_snapshots(&self) -> RepoResult<Vec<RobotSnapshot>>;

    /// Return the ID of the most recently seen robot.
    async fn most_recent(&self) -> RepoResult<Option<RobotId>>;

    /// Persist one telemetry frame to the rolling log (2-day expiry).
    async fn insert_log(&self, id: &RobotId, frame: &TelemetryFrame) -> RepoResult<()>;

    /// Count log entries for a robot (for pagination metadata).
    async fn count_logs(&self, id: &RobotId) -> RepoResult<i64>;

    /// Return a page of log entries, newest first.
    async fn query_logs(
        &self,
        id: &RobotId,
        limit: i64,
        offset: i64,
    ) -> RepoResult<Vec<LogEntry>>;
}
