//! Domain layer — pure value objects, aggregates, and events.
//!
//! Zero infrastructure imports.  All types in this module can be used in
//! unit tests without a database, network, or async runtime.

pub mod command;
pub mod event;
pub mod robot_id;
pub mod snapshot;
pub mod telemetry;

pub use command::RobotCommand;
pub use event::TelemetryEvent;
pub use robot_id::RobotId;
pub use snapshot::{LogEntry, RobotSnapshot};
pub use telemetry::TelemetryFrame;
