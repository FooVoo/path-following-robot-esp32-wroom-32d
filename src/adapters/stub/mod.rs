//! Stub adapters — compile-time fakes used in the Wokwi simulation binary.
//!
//! These adapters implement the port traits without any HAL dependency.
//! They are only compiled when the `sim` feature flag is active.

pub mod distance;
pub use distance::StubDistance;
