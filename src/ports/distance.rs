//! `DistancePort` — a single distance sensor (e.g. TF-Luna LIDAR).

/// Abstraction over one forward-facing distance sensor.
pub trait DistancePort {
    /// Drain any pending bytes from the sensor into the internal parser.
    ///
    /// Call once per main-loop tick.  Non-blocking; returns immediately if no
    /// bytes are available.
    fn poll(&mut self);

    /// Return the most recent valid distance reading in centimetres, or `None`
    /// if no valid frame has been received yet or the last reading is stale.
    fn distance_cm(&self) -> Option<u16>;

    /// Advance the internal staleness counter by one tick.
    ///
    /// Call once per main-loop tick, *after* `poll()`.
    fn tick_staleness(&mut self);
}
