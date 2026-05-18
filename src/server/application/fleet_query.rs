//! `FleetQueryService` — read-side queries: list robots, get snapshot, logs.

use std::sync::Arc;

use crate::server::{
    domain::{LogEntry, RobotId, RobotSnapshot},
    ports::{FleetRepository, fleet_repository::RepoResult},
};

pub struct FleetQueryService<R> {
    repo: Arc<R>,
}

impl<R: FleetRepository> FleetQueryService<R> {
    pub fn new(repo: Arc<R>) -> Self {
        Self { repo }
    }

    pub async fn list_robots(&self) -> RepoResult<Vec<RobotSnapshot>> {
        self.repo.list_snapshots().await
    }

    pub async fn get_robot(&self, id: &RobotId) -> RepoResult<Option<RobotSnapshot>> {
        self.repo.get_snapshot(id).await
    }

    pub async fn most_recent_robot(&self) -> RepoResult<Option<RobotSnapshot>> {
        match self.repo.most_recent().await? {
            Some(id) => self.repo.get_snapshot(&id).await,
            None     => Ok(None),
        }
    }

    pub async fn query_logs(
        &self,
        id: &RobotId,
        limit: i64,
        offset: i64,
    ) -> RepoResult<(Vec<LogEntry>, i64)> {
        let (logs, total) = tokio::join!(
            self.repo.query_logs(id, limit, offset),
            self.repo.count_logs(id),
        );
        Ok((logs?, total?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{
        domain::{RobotSnapshot, TelemetryFrame},
        ports::fleet_repository::RepoResult,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use std::{
        collections::HashMap,
        sync::Mutex,
    };

    struct StubRepo(Mutex<HashMap<String, RobotSnapshot>>);

    impl StubRepo {
        fn with(snaps: Vec<RobotSnapshot>) -> Arc<Self> {
            let map = snaps.into_iter().map(|s| (s.id.as_str().into(), s)).collect();
            Arc::new(Self(Mutex::new(map)))
        }
    }

    #[async_trait]
    impl FleetRepository for StubRepo {
        async fn save_snapshot(&self, snap: &RobotSnapshot) -> RepoResult<()> {
            self.0.lock().unwrap().insert(snap.id.as_str().into(), snap.clone());
            Ok(())
        }
        async fn get_snapshot(&self, id: &RobotId) -> RepoResult<Option<RobotSnapshot>> {
            Ok(self.0.lock().unwrap().get(id.as_str()).cloned())
        }
        async fn list_snapshots(&self) -> RepoResult<Vec<RobotSnapshot>> {
            let mut v: Vec<_> = self.0.lock().unwrap().values().cloned().collect();
            v.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
            Ok(v)
        }
        async fn most_recent(&self) -> RepoResult<Option<RobotId>> {
            Ok(self.0.lock().unwrap().values().max_by_key(|s| s.last_seen).map(|s| s.id.clone()))
        }
        async fn insert_log(&self, _: &RobotId, _: &TelemetryFrame) -> RepoResult<()> { Ok(()) }
        async fn count_logs(&self, _: &RobotId) -> RepoResult<i64> { Ok(0) }
        async fn query_logs(&self, _: &RobotId, _: i64, _: i64) -> RepoResult<Vec<LogEntry>> { Ok(vec![]) }
    }

    fn snap(ip: &str) -> RobotSnapshot {
        RobotSnapshot::placeholder(RobotId::new(ip), Utc::now())
    }

    #[tokio::test]
    async fn list_robots_returns_all() {
        let repo = StubRepo::with(vec![snap("1.1.1.1"), snap("2.2.2.2")]);
        let svc  = FleetQueryService::new(repo);
        let list = svc.list_robots().await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn get_robot_returns_none_for_unknown() {
        let repo = StubRepo::with(vec![]);
        let svc  = FleetQueryService::new(repo);
        let r = svc.get_robot(&RobotId::new("3.3.3.3")).await.unwrap();
        assert!(r.is_none());
    }

    #[tokio::test]
    async fn most_recent_robot_returns_latest() {
        use std::time::Duration;
        let mut s1 = snap("1.1.1.1");
        let mut s2 = snap("2.2.2.2");
        // s2 seen more recently
        s2.last_seen = s1.last_seen + chrono::Duration::seconds(10);

        let repo = StubRepo::with(vec![s1, s2]);
        let svc  = FleetQueryService::new(repo);
        let recent = svc.most_recent_robot().await.unwrap().unwrap();
        assert_eq!(recent.id.as_str(), "2.2.2.2");
    }
}
