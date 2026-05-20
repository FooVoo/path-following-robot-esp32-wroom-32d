//! Path-following robot firmware library (ESP32-WROOM-32D, `no_std`).
//!
//! # Architecture — Hexagonal / Ports-and-Adapters
//!
//! ```text
//!              ┌─────────────────────────────────────────────┐
//!              │                  domain/                    │
//!              │       state · path · robot (FSM)            │
//!              └────────────────┬────────────────────────────┘
//!                               │  uses traits
//!              ┌────────────────▼────────────────────────────┐
//!              │                  ports/                     │
//!              │  MotorPort  · DistancePort · InputPort      │
//!              │  DisplayPort · StepperPort                  │
//!              │  RemoteControlPort · TelemetryPort          │
//!              └────────────────┬────────────────────────────┘
//!                               │  implemented by
//!              ┌────────────────▼────────────────────────────┐
//!              │            adapters/esp32/                  │
//!              │  Drv8833 · Vl53l0xOnMux · Tca9548a          │
//!              │  Lcd1602 · Uln2003 · Joystick · WifiAdapter │
//!              └─────────────────────────────────────────────┘
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

// Adapters depend on `esp-hal` and are only compiled when targeting an ESP
// chip.  The ESP32 uses Xtensa (xtensa-esp32-none-elf) and the ESP32-C3 uses
// RISC-V (riscv32imc-unknown-none-elf); both are handled by the same adapter
// module since esp-hal abstractions are chip-agnostic at the API level.
// Host test builds (aarch64-apple-darwin etc.) exclude this module entirely so
// that `cargo test --lib` works without the ESP toolchain.
#[cfg(any(target_arch = "xtensa", target_arch = "riscv32"))]
pub mod adapters;

// Fleet-management server — host only, behind the `host-server` feature flag.
#[cfg(feature = "host-server")]
pub mod server;
