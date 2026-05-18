//! `RobotSnapshot` — aggregate root tracking the current state of one robot.

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::{RobotId, TelemetryFrame};

/// Paginated log entry returned by the logs query.
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub id: i64,
    pub received_at: DateTime<Utc>,
    pub frame: TelemetryFrame,
}

/// In-memory aggregate: current state + statistics for one robot.
#[derive(Debug, Clone, Serialize)]
pub struct RobotSnapshot {
    /// Stable robot identity.
    pub id: RobotId,
    /// Most recently received telemetry frame.
    pub latest: TelemetryFrame,
    /// Timestamp when the most recent frame was received by the server.
    pub last_seen: DateTime<Utc>,
    /// Total number of telemetry frames received in this session.
    pub received_count: u64,
}

impl RobotSnapshot {
    /// Create a new snapshot seeded from a real telemetry frame.
    pub fn new(frame: TelemetryFrame, now: DateTime<Utc>) -> Self {
        Self {
            id: frame.robot_id.clone(),
            latest: frame,
            last_seen: now,
            received_count: 1,
        }
    }

    /// Create an empty placeholder (used to pre-seed a known IP before any
    /// frame arrives, e.g. from the `ROBOT_IP` env var).
    pub fn placeholder(id: RobotId, now: DateTime<Utc>) -> Self {
        let placeholder_frame = TelemetryFrame {
            state: "UNKNOWN".into(),
            lidar_left_cm: None,
            lidar_right_cm: None,
            throttle_left: 0,
            throttle_right: 0,
            uptime_ms: 0,
            robot_id: id.clone(),
        };
        Self {
            id,
            latest: placeholder_frame,
            last_seen: now,
            received_count: 0,
        }
    }

    /// Incorporate a new telemetry frame.
    pub fn update(&mut self, frame: TelemetryFrame, now: DateTime<Utc>) {
        self.latest = frame;
        self.last_seen = now;
        self.received_count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(state: &str, ip: &str) -> TelemetryFrame {
        TelemetryFrame {
            state: state.into(),
            lidar_left_cm: None,
            lidar_right_cm: None,
            throttle_left: 0,
            throttle_right: 0,
            uptime_ms: 0,
            robot_id: RobotId::new(ip),
        }
    }

    #[test]
    fn new_snapshot_has_count_one() {
        let snap = RobotSnapshot::new(frame("IDLE", "1.2.3.4"), Utc::now());
        assert_eq!(snap.received_count, 1);
        assert_eq!(snap.latest.state, "IDLE");
    }

    #[test]
    fn update_increments_count() {
        let mut snap = RobotSnapshot::new(frame("IDLE", "1.2.3.4"), Utc::now());
        snap.update(frame("PLAY", "1.2.3.4"), Utc::now());
        assert_eq!(snap.received_count, 2);
        assert_eq!(snap.latest.state, "PLAY");
    }

    #[test]
    fn placeholder_has_zero_count() {
        let snap = RobotSnapshot::placeholder(RobotId::new("5.6.7.8"), Utc::now());
        assert_eq!(snap.received_count, 0);
        assert_eq!(snap.latest.state, "UNKNOWN");
    }
}
