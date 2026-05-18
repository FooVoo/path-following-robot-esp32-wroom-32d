//! Robot FSM states and auxiliary enumerations.
//!
//! Pure data — no hardware dependencies.

/// Top-level state of the path-following robot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RobotState {
    /// Waiting for the operator to press the record button.
    Idle,
    /// Joystick commands are being recorded into the path buffer.
    Record,
    /// Recording complete; ready to play back.
    Ready,
    /// Replaying the recorded path.
    Play,
    /// An obstacle was detected during playback; executing avoidance manoeuvre.
    Avoiding,
    /// Path buffer overflowed; robot is halted and must be power-cycled.
    Halt,
}

impl RobotState {
    /// Return the ASCII state name used in telemetry frames and log output.
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle     => "IDLE",
            Self::Record   => "RECORD",
            Self::Ready    => "READY",
            Self::Play     => "PLAY",
            Self::Avoiding => "AVOIDING",
            Self::Halt     => "HALT",
        }
    }
}

/// Which side an obstacle was detected on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObstacleSide {
    Left,
    Right,
    Both,
}
