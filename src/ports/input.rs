//! `InputPort` — joystick + button operator interface.

/// Abstraction over the operator input device (joystick + button).
///
/// The implementation is responsible for ADC sampling, axis mixing, and button
/// debouncing.  All timing-dependent logic uses `now_ms` (milliseconds since
/// boot) passed in from the caller so the domain layer never imports any HAL
/// time primitives.
pub trait InputPort {
    /// Sample ADC channels and update the debounced button state.
    ///
    /// `now_ms` — milliseconds elapsed since boot (for debounce logic).
    fn poll(&mut self, now_ms: u64);

    /// Signed throttle for the left motor, range `[-100, 100]`.
    fn throttle_left(&self) -> i8;

    /// Signed throttle for the right motor, range `[-100, 100]`.
    fn throttle_right(&self) -> i8;

    /// Returns `true` exactly once per confirmed button press edge.
    ///
    /// The flag is cleared after this call.
    fn take_button_press(&mut self) -> bool;

    /// Returns `true` while the button is physically held down.
    ///
    /// Unlike [`take_button_press`], this is a raw level query — it fires on
    /// every tick the button is held and never consumes any state.  Used by
    /// the domain layer to distinguish a long press (→ DIRECT) from a short
    /// press (→ RECORD) in the IDLE state.
    ///
    /// The default implementation returns `false`, which is correct for test
    /// mocks that only model instantaneous presses and for adapters that
    /// cannot report raw GPIO level.
    fn is_button_held(&self) -> bool {
        false
    }
}
