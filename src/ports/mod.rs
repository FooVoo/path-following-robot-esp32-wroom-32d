//! Port traits — the hexagonal boundary between the domain and hardware adapters.
//!
//! Each trait models one hardware *capability* that the robot domain needs.
//! Nothing in this module imports `esp_hal`; that is the adapters' concern.

pub mod distance;
pub mod input;
pub mod motors;
pub mod remote_control;
pub mod telemetry;
