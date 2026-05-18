//! `InMemoryFleetRepository` — in-process repository for tests and no-DB mode.

use std::{
    collections::HashMap,
    sync::Arc,
};

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;

use crate::server::{
    domain::{LogEntry, RobotId, RobotSnapshot, TelemetryFrame},
    ports::fleet_repository::{FleetRepository, RepoResult, RepositoryError},
};

/// An in-memory fleet repository suitable for tests and no-DB mode.
///
/// Thread-safe via `Arc<RwLock<_>>` internally; cheap to clone.
#[derive(Clone, Default)]
pub struct InMemoryFleetRepository {
    inner: Arc<RwLock<Inner>>,
}

#[derive(Default)]
struct Inner {
    snapshots: HashMap<String, RobotSnapshot>,
    /// Append-only log; entries are never pruned in this implementation.
    logs: Vec<(String, LogEntry)>,
    next_id: i64,
}

impl InMemoryFleetRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl FleetRepository for InMemoryFleetRepository {
    async fn save_snapshot(&self, snap: &RobotSnapshot) -> RepoResult<()> {
        self.inner
            .write()
            .await
            .snapshots
            .insert(snap.id.as_str().to_owned(), snap.clone());
        Ok(())
    }

    async fn get_snapshot(&self, id: &RobotId) -> RepoResult<Option<RobotSnapshot>> {
        Ok(self.inner.read().await.snapshots.get(id.as_str()).cloned())
    }

    async fn list_snapshots(&self) -> RepoResult<Vec<RobotSnapshot>> {
        let mut v: Vec<_> = self.inner.read().await.snapshots.values().cloned().collect();
        v.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
        Ok(v)
    }

    async fn most_recent(&self) -> RepoResult<Option<RobotId>> {
        Ok(self
            .inner
            .read()
            .await
            .snapshots
            .values()
            .max_by_key(|s| s.last_seen)
            .map(|s| s.id.clone()))
    }

    async fn insert_log(&self, id: &RobotId, frame: &TelemetryFrame) -> RepoResult<()> {
        let mut w = self.inner.write().await;
        let log_id = { w.next_id += 1; w.next_id };
        w.logs.push((
            id.as_str().to_owned(),
            LogEntry { id: log_id, received_at: Utc::now(), frame: frame.clone() },
        ));
        Ok(())
    }

    async fn count_logs(&self, id: &RobotId) -> RepoResult<i64> {
        let count = self
            .inner
            .read()
            .await
            .logs
            .iter()
            .filter(|(ip, _)| ip == id.as_str())
            .count();
        Ok(count as i64)
    }

    async fn query_logs(
        &self,
        id: &RobotId,
        limit: i64,
        offset: i64,
    ) -> RepoResult<Vec<LogEntry>> {
        let r = self.inner.read().await;
        let entries: Vec<LogEntry> = r
            .logs
            .iter()
            .filter(|(ip, _)| ip == id.as_str())
            .rev()                      // newest first
            .skip(offset as usize)
            .take(limit as usize)
            .map(|(_, e)| e.clone())
            .collect();
        Ok(entries)
    }
}
