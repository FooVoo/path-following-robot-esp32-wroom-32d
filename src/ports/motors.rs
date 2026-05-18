//! `MotorPort` — drive two brushed-DC motors with signed throttle values.

/// Drive two motors independently.
///
/// `left` and `right` are signed throttle values in the range `[-100, 100]`.
/// Positive values → forward; negative values → reverse; zero → coast.
pub trait MotorPort {
    /// Apply throttle to both motors simultaneously.
    fn drive(&mut self, left: i8, right: i8);

    /// Immediately coast both motors (remove PWM drive).
    fn coast(&mut self);
}
