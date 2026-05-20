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
    /// Direct joystick control — all axis values pass straight to the motors.
    ///
    /// Entered by holding the physical button for ≥ `LONG_PRESS_MS` from `Idle`.
    /// Exited back to `Idle` by a short button press.
    Direct,
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
            Self::Direct   => "DIRECT",
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
