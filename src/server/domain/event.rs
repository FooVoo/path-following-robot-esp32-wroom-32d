//! `TelemetryEvent` — domain event published to SSE subscribers after ingestion.

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::{RobotId, TelemetryFrame};

/// Published on every successfully ingested telemetry frame.
///
/// The event bus serialises this to JSON and broadcasts it to all active
/// SSE connections.
#[derive(Debug, Clone, Serialize)]
pub struct TelemetryEvent {
    /// Identity of the reporting robot.
    pub robot_id: RobotId,
    /// Decoded telemetry frame.
    pub frame: TelemetryFrame,
    /// Server-side reception timestamp (UTC).
    pub timestamp: DateTime<Utc>,
}
