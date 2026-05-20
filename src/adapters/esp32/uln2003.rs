//! ULN2003 stepper-motor driver adapter (28BYJ-48, half-step mode).
//!
//! The ULN2003 breakout accepts four TTL control lines IN1–IN4.  This adapter
//! drives a 28BYJ-48 unipolar motor using the 8-phase half-step sequence for
//! smooth, low-vibration motion.
//!
//! # Motor specification (28BYJ-48)
//!
//! | Property | Value |
//! |----------|-------|
//! | Steps per revolution (internal rotor) | 32 |
//! | Gear ratio | 1 : 63.684 |
//! | Half-steps per output shaft revolution | ~512 |
//! | Full half-step cycles to rotate 360° | 512 / 8 = 64 cycles |
//!
//! At `STEPPER_STEP_DELAY_US = 2 000` µs the shaft rotates at ≈15 rpm.
//!
//! # Half-step phase table
//!
//! ```text
//! Phase │ IN1  IN2  IN3  IN4
//! ──────┼────────────────────
//!   0   │  1    0    0    0
//!   1   │  1    1    0    0
//!   2   │  0    1    0    0
//!   3   │  0    1    1    0
//!   4   │  0    0    1    0
//!   5   │  0    0    1    1
//!   6   │  0    0    0    1
//!   7   │  1    0    0    1
//! ```

use esp_hal::{
    delay::Delay,
    gpio::{Level, Output},
};

use crate::{config::STEPPER_STEP_DELAY_US, ports::stepper::StepperPort};

// ── Phase table ───────────────────────────────────────────────────────────────

/// Half-step coil pattern per phase: `[IN1, IN2, IN3, IN4]`.
pub(crate) const HALF_STEP: [[bool; 4]; 8] = [
    [true,  false, false, false],
    [true,  true,  false, false],
    [false, true,  false, false],
    [false, true,  true,  false],
    [false, false, true,  false],
    [false, false, true,  true ],
    [false, false, false, true ],
    [true,  false, false, true ],
];

// ── Adapter ───────────────────────────────────────────────────────────────────

/// ULN2003 stepper driver for a 28BYJ-48 motor.
pub struct Uln2003<'d> {
    in1:   Output<'d>,
    in2:   Output<'d>,
    in3:   Output<'d>,
    in4:   Output<'d>,
    delay: Delay,
    phase: usize,
}

impl<'d> Uln2003<'d> {
    /// Create the adapter.
    ///
    /// Pins must be push-pull outputs (typically initialised `Level::Low`).
    pub fn new(
        in1: Output<'d>,
        in2: Output<'d>,
        in3: Output<'d>,
        in4: Output<'d>,
    ) -> Self {
        Self {
            in1,
            in2,
            in3,
            in4,
            delay: Delay::new(),
            phase: 0,
        }
    }

    /// Apply the coil energisation pattern for the current phase index.
    fn apply_phase(&mut self) {
        let p = HALF_STEP[self.phase];
        self.in1.set_level(Level::from(p[0]));
        self.in2.set_level(Level::from(p[1]));
        self.in3.set_level(Level::from(p[2]));
        self.in4.set_level(Level::from(p[3]));
    }
}

impl<'d> StepperPort for Uln2003<'d> {
    fn step(&mut self, steps: i32) {
        let forward = steps >= 0;
        let count   = steps.unsigned_abs() as u32;

        for _ in 0..count {
            if forward {
                self.phase = (self.phase + 1) % 8;
            } else {
                self.phase = (self.phase + 7) % 8; // wraps backward without underflow
            }
            self.apply_phase();
            self.delay.delay_micros(STEPPER_STEP_DELAY_US);
        }
    }

    fn release(&mut self) {
        self.in1.set_level(Level::Low);
        self.in2.set_level(Level::Low);
        self.in3.set_level(Level::Low);
        self.in4.set_level(Level::Low);
    }
}

// ── Unit tests (run on host — phase arithmetic only) ─────────────────────────

#[cfg(test)]
mod tests {
    use super::HALF_STEP;

    /// Every half-step phase must energise exactly 1 or 2 coils.
    #[test]
    fn half_step_coil_count_is_one_or_two() {
        for (i, row) in HALF_STEP.iter().enumerate() {
            let active = row.iter().filter(|&&x| x).count();
            assert!(
                active == 1 || active == 2,
                "phase {i}: expected 1 or 2 active coils, got {active}"
            );
        }
    }

    /// Stepping forward 8 phases returns to phase 0.
    #[test]
    fn forward_eight_steps_wraps_to_zero() {
        let mut phase = 0usize;
        for _ in 0..8 {
            phase = (phase + 1) % 8;
        }
        assert_eq!(phase, 0);
    }

    /// Stepping backward 8 phases returns to phase 0.
    #[test]
    fn backward_eight_steps_wraps_to_zero() {
        let mut phase = 0usize;
        for _ in 0..8 {
            phase = (phase + 7) % 8;
        }
        assert_eq!(phase, 0);
    }

    /// Backward step from phase 0 goes to phase 7 (no underflow).
    #[test]
    fn backward_from_zero_is_seven() {
        let phase = (0usize + 7) % 8;
        assert_eq!(phase, 7);
    }
}
