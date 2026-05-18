//! `BroadcastEventPublisher` — SSE fan-out via a tokio broadcast channel.

use tokio::sync::broadcast;

use crate::server::{
    domain::TelemetryEvent,
    ports::EventPublisher,
};

/// Broadcasts serialised `TelemetryEvent` JSON strings to all SSE subscribers.
///
/// Cloning is cheap (the channel is `Arc`-backed internally).
#[derive(Clone)]
pub struct BroadcastEventPublisher {
    tx: broadcast::Sender<String>,
}

impl BroadcastEventPublisher {
    /// Create a new publisher with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }
}

impl EventPublisher for BroadcastEventPublisher {
    fn publish(&self, event: &TelemetryEvent) {
        // Serialise to JSON once; broadcast to all subscribers.
        // Errors (no subscribers, lagged receivers) are silently ignored.
        if let Ok(json) = serde_json::to_string(event) {
            let _ = self.tx.send(json);
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}
