//! Stub distance adapter for Wokwi simulation.
//!
//! [`StubDistance`] implements [`crate::ports::distance::DistancePort`] without
//! any I2C or HAL dependency.  It oscillates between a safe distance and an
//! obstacle distance to exercise the robot's avoidance FSM in the simulator.
//!
//! # Oscillation cycle (at 100 Hz loop rate)
//!
//! | Phase    | Length              | Value   | Robot effect               |
//! |----------|---------------------|---------|----------------------------|
//! | Safe     | 400 ticks (4 s)     | 200 cm  | Drives freely              |
//! | Obstacle | 100 ticks (1 s)     | 50 cm   | Below `OBSTACLE_CM=80` →   |
//! |          |                     |         | triggers `AVOIDING` state  |
//!
//! The cycle repeats indefinitely so the avoidance FSM is exercised every
//! 5 seconds once the robot is in `PLAY` state.

use crate::{config::STALE_TICKS, ports::distance::DistancePort};

/// Number of ticks to emit a safe distance.
const SAFE_TICKS: u32 = 400;
/// Number of ticks to emit an obstacle distance.
const OBSTACLE_TICKS: u32 = 100;
/// Total cycle length.
const CYCLE: u32 = SAFE_TICKS + OBSTACLE_TICKS;
/// Safe distance (cm) — well above `CLEAR_CM = 100`.
const SAFE_CM: u16 = 200;
/// Obstacle distance (cm) — well below `OBSTACLE_CM = 80`.
const STUB_OBSTACLE_CM: u16 = 50;

/// Stub distance adapter — oscillates between safe and obstacle readings.
///
/// Intended for the Wokwi simulation binary only (`src/bin/main_sim.rs`).
/// Because no I2C hardware is involved, this compiles and runs on any target.
pub struct StubDistance {
    /// Tick counter within the current oscillation cycle.
    tick:        u32,
    /// Most recent distance reading (cm); `None` when declared stale.
    value_cm:    Option<u16>,
    /// Staleness counter; reset to 0 on every successful poll.
    stale_ticks: u32,
}

impl StubDistance {
    /// Construct a new stub starting in the safe phase (200 cm).
    pub fn new() -> Self {
        Self {
            tick:        0,
            value_cm:    Some(SAFE_CM),
            stale_ticks: 0,
        }
    }
}

impl DistancePort for StubDistance {
    fn poll(&mut self) {
        self.tick = (self.tick + 1) % CYCLE;
        self.value_cm = if self.tick < SAFE_TICKS {
            Some(SAFE_CM)
        } else {
            Some(STUB_OBSTACLE_CM)
        };
        // Stub always produces valid data — reset staleness so the domain
        // never sees a stale reading caused by the oscillator itself.
        self.stale_ticks = 0;
    }

    fn distance_cm(&self) -> Option<u16> {
        self.value_cm
    }

    fn tick_staleness(&mut self) {
        self.stale_ticks = self.stale_ticks.saturating_add(1);
        if self.stale_ticks >= STALE_TICKS {
            self.value_cm = None;
        }
    }
}
