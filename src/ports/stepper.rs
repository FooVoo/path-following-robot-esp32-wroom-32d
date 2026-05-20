//! Stepper-motor output port.
//!
//! Implemented by [`crate::adapters::esp32::uln2003::Uln2003`] for the
//! ULN2003 / 28BYJ-48 combination in half-step mode.

/// Stepper-motor control port.
pub trait StepperPort {
    /// Step the motor by `steps` half-steps.
    ///
    /// Positive values rotate forward; negative values rotate in reverse.
    /// This call **blocks** until all steps are complete.
    fn step(&mut self, steps: i32);

    /// De-energise all coils.
    ///
    /// The shaft may drift under external load.  Call after a move sequence
    /// to save power and reduce heat.
    fn release(&mut self);
}
