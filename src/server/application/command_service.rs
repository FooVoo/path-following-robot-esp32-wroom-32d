//! `CommandService` — resolve target robot and dispatch a `RobotCommand`.

use std::sync::Arc;

use crate::server::{
    domain::{RobotCommand, RobotId},
    ports::{
        CommandGateway,
        FleetRepository,
        command_gateway::GatewayError,
        fleet_repository::RepoResult,
    },
};

#[derive(Debug)]
pub enum CommandError {
    NoRobot,
    Repo(RepoResult<()>),
    Gateway(GatewayError),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRobot       => write!(f, "no robot available"),
            Self::Repo(_e)      => write!(f, "repository error"),
            Self::Gateway(e)    => write!(f, "gateway error: {e}"),
        }
    }
}

pub struct CommandService<R, G> {
    repo:    Arc<R>,
    gateway: Arc<G>,
}

impl<R: FleetRepository, G: CommandGateway> CommandService<R, G> {
    pub fn new(repo: Arc<R>, gateway: Arc<G>) -> Self {
        Self { repo, gateway }
    }

    /// Send `command` to a specific robot IP.
    pub async fn send_to(&self, id: &RobotId, command: RobotCommand) -> Result<(), CommandError> {
        self.gateway
            .send(id, command)
            .await
            .map_err(CommandError::Gateway)
    }

    /// Send `command` to the most recently active robot.
    pub async fn send_to_recent(&self, command: RobotCommand) -> Result<(), CommandError> {
        let id = self
            .repo
            .most_recent()
            .await
            .map_err(|_| CommandError::NoRobot)?
            .ok_or(CommandError::NoRobot)?;
        self.send_to(&id, command).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{
        domain::{LogEntry, RobotSnapshot, TelemetryFrame},
        ports::{
            command_gateway::GatewayError,
            fleet_repository::{RepoResult, RepositoryError},
        },
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::Mutex;

    // ── Stubs ─────────────────────────────────────────────────────────────────

    struct StubRepo(Option<RobotId>);

    #[async_trait]
    impl FleetRepository for StubRepo {
        async fn save_snapshot(&self, _: &RobotSnapshot) -> RepoResult<()> { Ok(()) }
        async fn get_snapshot(&self, _: &RobotId) -> RepoResult<Option<RobotSnapshot>> { Ok(None) }
        async fn list_snapshots(&self) -> RepoResult<Vec<RobotSnapshot>> { Ok(vec![]) }
        async fn most_recent(&self) -> RepoResult<Option<RobotId>> { Ok(self.0.clone()) }
        async fn insert_log(&self, _: &RobotId, _: &TelemetryFrame) -> RepoResult<()> { Ok(()) }
        async fn count_logs(&self, _: &RobotId) -> RepoResult<i64> { Ok(0) }
        async fn query_logs(&self, _: &RobotId, _: i64, _: i64) -> RepoResult<Vec<LogEntry>> { Ok(vec![]) }
    }

    struct SpyGateway {
        calls: Mutex<Vec<(RobotId, RobotCommand)>>,
        fail:  bool,
    }

    impl SpyGateway {
        fn ok()   -> Arc<Self> { Arc::new(Self { calls: Mutex::new(vec![]), fail: false }) }
        fn fail() -> Arc<Self> { Arc::new(Self { calls: Mutex::new(vec![]), fail: true  }) }
        fn calls(&self) -> Vec<(RobotId, RobotCommand)> { self.calls.lock().unwrap().clone() }
    }

    #[async_trait]
    impl CommandGateway for SpyGateway {
        async fn send(&self, target: &RobotId, command: RobotCommand) -> Result<(), GatewayError> {
            if self.fail {
                return Err(GatewayError::Send("simulated".into()));
            }
            self.calls.lock().unwrap().push((target.clone(), command));
            Ok(())
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn send_to_recent_dispatches_when_robot_known() {
        let repo = Arc::new(StubRepo(Some(RobotId::new("1.2.3.4"))));
        let gw   = SpyGateway::ok();
        let svc  = CommandService::new(repo, Arc::clone(&gw));

        svc.send_to_recent(RobotCommand::Button).await.unwrap();

        let calls = gw.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0.as_str(), "1.2.3.4");
        assert_eq!(calls[0].1, RobotCommand::Button);
    }

    #[tokio::test]
    async fn send_to_recent_errors_when_no_robot() {
        let repo = Arc::new(StubRepo(None));
        let gw   = SpyGateway::ok();
        let svc  = CommandService::new(repo, gw);

        let result = svc.send_to_recent(RobotCommand::Button).await;
        assert!(matches!(result, Err(CommandError::NoRobot)));
    }

    #[tokio::test]
    async fn send_to_propagates_gateway_error() {
        let repo = Arc::new(StubRepo(Some(RobotId::new("1.1.1.1"))));
        let gw   = SpyGateway::fail();
        let svc  = CommandService::new(repo, gw);

        let result = svc.send_to(&RobotId::new("1.1.1.1"), RobotCommand::Button).await;
        assert!(matches!(result, Err(CommandError::Gateway(_))));
    }
}
