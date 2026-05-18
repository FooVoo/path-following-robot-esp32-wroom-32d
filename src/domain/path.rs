//! Path recording and playback buffer.
//!
//! A `PathCommand` is a snapshot of the motor throttle at a given time; the
//! replay engine applies each command in sequence for `duration_ms`.

use heapless::Vec;

/// One recorded motion segment.
#[derive(Clone, Copy, Debug)]
pub struct PathCommand {
    /// Signed throttle for the left motor  (`-100 … 100`).
    pub throttle_l: i8,
    /// Signed throttle for the right motor (`-100 … 100`).
    pub throttle_r: i8,
    /// How long this segment should play back (milliseconds).
    pub duration_ms: u16,
}

/// Maximum number of commands that can be recorded in one path.
pub const PATH_CAPACITY: usize = 512;

/// Fixed-capacity path storage.
pub type PathBuffer = Vec<PathCommand, PATH_CAPACITY>;
