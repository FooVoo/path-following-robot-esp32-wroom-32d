//! `RobotCommand` — value object for commands sent to a robot over UDP.

/// A command sent to a robot.
#[derive(Debug, Clone, PartialEq)]
pub enum RobotCommand {
    /// Simulate a button press (cycles the FSM: Idle→Record→Ready→Play).
    Button,
    /// Set motor throttle.  Both values are in `[-100, 100]`.
    Throttle { left: i8, right: i8 },
}

impl RobotCommand {
    /// Encode to the 4-byte wire format `[0xA5, type, v1, v2]`.
    pub fn to_wire(&self) -> [u8; 4] {
        match self {
            Self::Button                => [0xA5, 0x02, 0, 0],
            Self::Throttle { left, right } => [0xA5, 0x01, *left as u8, *right as u8],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_wire_encoding() {
        assert_eq!(RobotCommand::Button.to_wire(), [0xA5, 0x02, 0x00, 0x00]);
    }

    #[test]
    fn throttle_wire_encoding() {
        let cmd = RobotCommand::Throttle { left: 50, right: -50 };
        let wire = cmd.to_wire();
        assert_eq!(wire[0], 0xA5);
        assert_eq!(wire[1], 0x01);
        assert_eq!(wire[2] as i8, 50);
        assert_eq!(wire[3] as i8, -50);
    }
}
