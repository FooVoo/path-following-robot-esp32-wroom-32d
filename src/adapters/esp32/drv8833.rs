//! DRV8833 dual-channel H-bridge motor driver adapter.
//!
//! The DRV8833 exposes two H-bridges, each controlled by two PWM pins:
//!
//! ```text
//! AIN1  AIN2  → motor A
//!  H     L    forward
//!  L     H    reverse
//!  L     L    coast
//!  H     H    brake (avoid — back-EMF spike risk)
//! ```
//!
//! We use fast-decay mode: one pin carries the PWM duty, the other stays LOW.
//! The duty is derived from a signed throttle value in `[-100, 100]`.
//!
//! # LEDC channel lifetime
//!
//! Each [`Channel<'a, LowSpeed>`] holds an internal reference `&'a dyn
//! TimerIFace<LowSpeed>`.  The `'a` lifetime in [`Drv8833<'a>`] is that
//! reference lifetime — the LEDC timer must outlive this struct.  In practice
//! both the timer and this adapter live for the entire `main()` run, so the
//! borrow checker will enforce the correct ordering at compile time.

use log::error;

use esp_hal::ledc::{
    LowSpeed,
    channel::{Channel, ChannelIFace},
};

use crate::ports::motors::MotorPort;

/// Maps a signed throttle value to a (forward_duty, reverse_duty) pair.
///
/// - positive → forward pin = duty%, reverse pin = 0%
/// - negative → forward pin = 0%,   reverse pin = duty%
/// - zero     → both = 0% (coast)
///
/// Clamps the input to `[-100, 100]` before conversion.
#[inline]
fn throttle_to_duties(t: i8) -> (u8, u8) {
    let t = t.clamp(-100, 100);
    if t >= 0 {
        (t as u8, 0)
    } else {
        (0, (-t) as u8)
    }
}

// ---------------------------------------------------------------------------

/// DRV8833 adapter: four LEDC channels (AIN1/AIN2/BIN1/BIN2).
pub struct Drv8833<'a> {
    /// Motor A (left) forward PWM.
    ain1: Channel<'a, LowSpeed>,
    /// Motor A (left) reverse PWM.
    ain2: Channel<'a, LowSpeed>,
    /// Motor B (right) forward PWM.
    bin1: Channel<'a, LowSpeed>,
    /// Motor B (right) reverse PWM.
    bin2: Channel<'a, LowSpeed>,
}

impl<'a> Drv8833<'a> {
    /// Create a new adapter from four pre-configured LEDC channels.
    ///
    /// The channels must already have been configured with a timer and the
    /// correct output pins before calling this constructor.
    pub fn new(
        ain1: Channel<'a, LowSpeed>,
        ain2: Channel<'a, LowSpeed>,
        bin1: Channel<'a, LowSpeed>,
        bin2: Channel<'a, LowSpeed>,
    ) -> Self {
        Self { ain1, ain2, bin1, bin2 }
    }

    /// Apply `(fwd, rev)` duties to one H-bridge half.
    fn apply_half(fwd: &Channel<'a, LowSpeed>, rev: &Channel<'a, LowSpeed>, duty_fwd: u8, duty_rev: u8) {
        if let Err(e) = fwd.set_duty(duty_fwd) {
            error!("LEDC set_duty fwd={} err={:?}", duty_fwd, e);
        }
        if let Err(e) = rev.set_duty(duty_rev) {
            error!("LEDC set_duty rev={} err={:?}", duty_rev, e);
        }
    }
}

impl<'a> MotorPort for Drv8833<'a> {
    /// Drive both motors.  `left` / `right` in `[-100, 100]`.
    fn drive(&mut self, left: i8, right: i8) {
        let (la, lb) = throttle_to_duties(left);
        let (ra, rb) = throttle_to_duties(right);
        Self::apply_half(&self.ain1, &self.ain2, la, lb);
        Self::apply_half(&self.bin1, &self.bin2, ra, rb);
    }

    /// Coast both motors (all PWM pins = 0%).
    fn coast(&mut self) {
        self.drive(0, 0);
    }
}

// ---------------------------------------------------------------------------
// Unit tests (run on host with `cargo test --lib`)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::throttle_to_duties;

    #[test]
    fn zero_is_coast() {
        assert_eq!(throttle_to_duties(0), (0, 0));
    }

    #[test]
    fn positive_forward() {
        assert_eq!(throttle_to_duties(50), (50, 0));
        assert_eq!(throttle_to_duties(100), (100, 0));
    }

    #[test]
    fn negative_reverse() {
        assert_eq!(throttle_to_duties(-50), (0, 50));
        assert_eq!(throttle_to_duties(-100), (0, 100));
    }

    #[test]
    fn clamps_above_100() {
        assert_eq!(throttle_to_duties(127), (100, 0));
    }

    #[test]
    fn clamps_below_minus_100() {
        assert_eq!(throttle_to_duties(-128), (0, 100));
    }
}
