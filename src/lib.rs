//! Path-following robot firmware library (ESP32-WROOM-32D, `no_std`).
//!
//! # Architecture — Hexagonal / Ports-and-Adapters
//!
//! ```text
//!              ┌─────────────────────────────────┐
//!              │           domain/               │
//!              │  state  · path  · robot (FSM)   │
//!              └────────────┬────────────────────┘
//!                           │  uses traits
//!              ┌────────────▼────────────────────┐
//!              │           ports/                │
//!              │  MotorPort · DistancePort        │
//!              │  InputPort                      │
//!              └────────────┬────────────────────┘
//!                           │  implemented by
//!              ┌────────────▼────────────────────┐
//!              │       adapters/esp32/            │
//!              │  Drv8833 · TfLuna · Joystick     │
//!              └─────────────────────────────────┘
//! ```
//!
//! The `domain` layer has **zero** `esp_hal` imports; all HAL calls live in
//! the adapter layer.  `src/bin/main.rs` is the composition root.

// `no_std` for embedded target; `std` re-enabled when compiling the test
// harness OR the host-side server (which depends on std crates like axum).
#![cfg_attr(not(any(test, feature = "host-server")), no_std)]

pub mod config;
pub mod domain;
pub mod ports;

// Protobuf generated code (prost). Available on all targets; `std` support
// is activated by the `host-server` feature (see Cargo.toml).
pub mod proto;

// Adapters depend on `esp-hal` and are only compiled when targeting the
// ESP32 / Xtensa hardware.  They are excluded from host test builds so that
// `cargo test --lib --target aarch64-apple-darwin` can run without the
// ESP32 toolchain constraints.
#[cfg(target_arch = "xtensa")]
pub mod adapters;

// Fleet-management server — host only, behind the `host-server` feature flag.
#[cfg(feature = "host-server")]
pub mod server;
