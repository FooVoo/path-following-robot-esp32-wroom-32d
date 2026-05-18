//! `IngestTelemetryService` — core use case: receive a frame, update state,
//! publish the SSE event, persist the log row.

use std::sync::Arc;

use chrono::Utc;
use tracing::{warn};

use crate::server::{
    domain::{RobotSnapshot, TelemetryFrame, TelemetryEvent},
    ports::{EventPublisher, FleetRepository, fleet_repository::RepoResult},
};

/// Processes one telemetry frame end-to-end.
pub struct IngestTelemetryService<R, E> {
    repo:   Arc<R>,
    events: Arc<E>,
}

impl<R, E> IngestTelemetryService<R, E>
where
    R: FleetRepository,
    E: EventPublisher,
{
    pub fn new(repo: Arc<R>, events: Arc<E>) -> Self {
        Self { repo, events }
    }

    /// Handle one decoded telemetry frame.
    ///
    /// Steps:
    /// 1. Load or create the robot's in-memory snapshot.
    /// 2. Persist the updated snapshot.
    /// 3. Insert a log row (fire-and-forget; errors are logged, not fatal).
    /// 4. Publish the SSE event.
    pub async fn ingest(&self, frame: TelemetryFrame) -> RepoResult<()> {
        let now  = Utc::now();
        let id   = frame.robot_id.clone();

        // Load or initialise the snapshot; only call update() on existing ones
        // (new() already captures the first frame and sets received_count = 1).
        let snapshot = match self.repo.get_snapshot(&id).await? {
            Some(mut existing) => { existing.update(frame.clone(), now); existing }
            None               => RobotSnapshot::new(frame.clone(), now),
        };
        self.repo.save_snapshot(&snapshot).await?;

        // Persist log row (non-fatal; robot keeps running if DB is down).
        if let Err(e) = self.repo.insert_log(&id, &frame).await {
            warn!("log insert failed for {id}: {e}");
        }

        // Fan-out to SSE clients.
        self.events.publish(&TelemetryEvent { robot_id: id, frame, timestamp: now });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use crate::server::{
        domain::{RobotId, TelemetryEvent},
        ports::fleet_repository::RepositoryError,
    };
    use async_trait::async_trait;
    use tokio::sync::broadcast;

    // ── Stubs ────────────────────────────────────────────────────────────────

    struct SpyRepo {
        snapshots: Mutex<std::collections::HashMap<String, RobotSnapshot>>,
        log_count: Mutex<usize>,
    }

    impl SpyRepo {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                snapshots: Mutex::new(Default::default()),
                log_count: Mutex::new(0),
            })
        }

        fn snapshot_count(&self) -> usize {
            self.snapshots.lock().unwrap().len()
        }

        fn log_count(&self) -> usize {
            *self.log_count.lock().unwrap()
        }
    }

    #[async_trait]
    impl FleetRepository for SpyRepo {
        async fn save_snapshot(&self, snap: &RobotSnapshot) -> crate::server::ports::fleet_repository::RepoResult<()> {
            self.snapshots.lock().unwrap().insert(snap.id.as_str().into(), snap.clone());
            Ok(())
        }
        async fn get_snapshot(&self, id: &RobotId) -> crate::server::ports::fleet_repository::RepoResult<Option<RobotSnapshot>> {
            Ok(self.snapshots.lock().unwrap().get(id.as_str()).cloned())
        }
        async fn list_snapshots(&self) -> crate::server::ports::fleet_repository::RepoResult<Vec<RobotSnapshot>> {
            Ok(self.snapshots.lock().unwrap().values().cloned().collect())
        }
        async fn most_recent(&self) -> crate::server::ports::fleet_repository::RepoResult<Option<RobotId>> {
            Ok(self.snapshots.lock().unwrap()
                .values()
                .max_by_key(|s| s.last_seen)
                .map(|s| s.id.clone()))
        }
        async fn insert_log(&self, _: &RobotId, _: &TelemetryFrame) -> crate::server::ports::fleet_repository::RepoResult<()> {
            *self.log_count.lock().unwrap() += 1;
            Ok(())
        }
        async fn count_logs(&self, _: &RobotId) -> crate::server::ports::fleet_repository::RepoResult<i64> { Ok(0) }
        async fn query_logs(&self, _: &RobotId, _: i64, _: i64) -> crate::server::ports::fleet_repository::RepoResult<Vec<crate::server::domain::LogEntry>> { Ok(vec![]) }
    }

    struct SpyBus {
        tx: broadcast::Sender<String>,
    }

    impl SpyBus {
        fn new() -> (Arc<Self>, broadcast::Receiver<String>) {
            let (tx, rx) = broadcast::channel(16);
            (Arc::new(Self { tx }), rx)
        }
    }

    impl EventPublisher for SpyBus {
        fn publish(&self, event: &TelemetryEvent) {
            let _ = self.tx.send(serde_json::to_string(event).unwrap());
        }
        fn subscribe(&self) -> broadcast::Receiver<String> {
            self.tx.subscribe()
        }
    }

    fn sample_frame(ip: &str) -> TelemetryFrame {
        TelemetryFrame {
            state: "PLAY".into(),
            lidar_left_cm: Some(100),
            lidar_right_cm: None,
            throttle_left: 50,
            throttle_right: 50,
            uptime_ms: 1000,
            robot_id: RobotId::new(ip),
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ingest_creates_snapshot_and_log() {
        let repo = SpyRepo::new();
        let (bus, _rx) = SpyBus::new();
        let svc = IngestTelemetryService::new(Arc::clone(&repo), Arc::clone(&bus));

        svc.ingest(sample_frame("1.2.3.4")).await.unwrap();

        assert_eq!(repo.snapshot_count(), 1);
        assert_eq!(repo.log_count(), 1);
    }

    #[tokio::test]
    async fn ingest_publishes_sse_event() {
        let repo = SpyRepo::new();
        let (bus, mut rx) = SpyBus::new();
        let svc = IngestTelemetryService::new(repo, Arc::clone(&bus));

        svc.ingest(sample_frame("1.2.3.4")).await.unwrap();

        let msg = rx.try_recv().expect("event not published");
        assert!(msg.contains("PLAY"));
        assert!(msg.contains("1.2.3.4"));
    }

    #[tokio::test]
    async fn second_ingest_updates_existing_snapshot() {
        let repo = SpyRepo::new();
        let (bus, _) = SpyBus::new();
        let svc = IngestTelemetryService::new(Arc::clone(&repo), bus);

        svc.ingest(sample_frame("1.2.3.4")).await.unwrap();
        let mut f2 = sample_frame("1.2.3.4");
        f2.state = "IDLE".into();
        svc.ingest(f2).await.unwrap();

        // Still one robot, but two log entries and updated state.
        assert_eq!(repo.snapshot_count(), 1);
        assert_eq!(repo.log_count(), 2);
        let snap = repo.get_snapshot(&RobotId::new("1.2.3.4")).await.unwrap().unwrap();
        assert_eq!(snap.latest.state, "IDLE");
        assert_eq!(snap.received_count, 2);
    }

    #[tokio::test]
    async fn db_failure_does_not_abort_ingest() {
        struct FailingRepo;

        #[async_trait]
        impl FleetRepository for FailingRepo {
            async fn save_snapshot(&self, _: &RobotSnapshot) -> crate::server::ports::fleet_repository::RepoResult<()> { Ok(()) }
            async fn get_snapshot(&self, _: &RobotId) -> crate::server::ports::fleet_repository::RepoResult<Option<RobotSnapshot>> { Ok(None) }
            async fn list_snapshots(&self) -> crate::server::ports::fleet_repository::RepoResult<Vec<RobotSnapshot>> { Ok(vec![]) }
            async fn most_recent(&self) -> crate::server::ports::fleet_repository::RepoResult<Option<RobotId>> { Ok(None) }
            async fn insert_log(&self, _: &RobotId, _: &TelemetryFrame) -> crate::server::ports::fleet_repository::RepoResult<()> {
                Err(RepositoryError::Database("simulated failure".into()))
            }
            async fn count_logs(&self, _: &RobotId) -> crate::server::ports::fleet_repository::RepoResult<i64> { Ok(0) }
            async fn query_logs(&self, _: &RobotId, _: i64, _: i64) -> crate::server::ports::fleet_repository::RepoResult<Vec<crate::server::domain::LogEntry>> { Ok(vec![]) }
        }

        let (bus, mut rx) = SpyBus::new();
        let svc = IngestTelemetryService::new(Arc::new(FailingRepo), bus);

        // Should not return Err even though insert_log fails.
        svc.ingest(sample_frame("9.9.9.9")).await.unwrap();
        // SSE event still published.
        assert!(rx.try_recv().is_ok());
    }
}
