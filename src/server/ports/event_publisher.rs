//! `EventPublisher` — domain event broadcast port (SSE fan-out).

use tokio::sync::broadcast;

use crate::server::domain::TelemetryEvent;

/// Publish telemetry events to all active SSE subscribers.
pub trait EventPublisher: Send + Sync {
    /// Publish an event.  Implementations must be non-blocking (fire-and-forget).
    fn publish(&self, event: &TelemetryEvent);

    /// Subscribe to the event stream.  Returns a `broadcast::Receiver<String>`
    /// where each message is the JSON-serialised `TelemetryEvent`.
    fn subscribe(&self) -> broadcast::Receiver<String>;
}
