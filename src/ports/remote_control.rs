//! `RemoteControlPort` — receive operator commands from a wireless remote.

/// Receive throttle overrides and button events from a remote source
/// (UDP over WiFi, BLE, serial, …).
pub trait RemoteControlPort {
    /// Pump the underlying transport layer forward by one tick.
    ///
    /// **Must be called once per `Robot::tick()`, before `poll_throttle()` or
    /// `poll_button()`.**
    ///
    /// `now_ms` — milliseconds since boot.  WiFi adapters use this to drive the
    /// smoltcp clock; simpler adapters may ignore it.
    fn poll_network(&mut self, now_ms: u64);

    /// Take any pending remote throttle override received since the last call.
    ///
    /// Returns `Some((left, right))` in `[-100, 100]` if a new command arrived;
    /// `None` otherwise.  Calling this clears the buffered value.
    fn poll_throttle(&mut self) -> Option<(i8, i8)>;

    /// Returns `true` exactly once if a remote button event arrived since the
    /// last call, then clears the flag.
    fn poll_button(&mut self) -> bool;
}
