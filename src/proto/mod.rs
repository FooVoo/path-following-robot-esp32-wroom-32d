//! Generated protobuf types for the robot ↔ server telemetry wire protocol.
//!
//! The `TelemetryFrame` message is encoded by the robot (ESP32) and decoded
//! by the host-side `telemetry-server`.  Both sides share this module.
//!
//! Do not edit manually — regenerate by running `cargo build`.

// prost-generated code uses `alloc` types when `no_std` is active.
#![allow(clippy::derive_partial_eq_without_eq)]

include!(concat!(env!("OUT_DIR"), "/robot_telemetry.rs"));
