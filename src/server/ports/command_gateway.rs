//! `CommandGateway` — outbound port for sending commands to robots.

use async_trait::async_trait;

use crate::server::domain::{RobotCommand, RobotId};

#[derive(Debug)]
pub enum GatewayError {
    UnknownRobot,
    BadAddress(String),
    Send(String),
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownRobot       => write!(f, "robot IP unknown"),
            Self::BadAddress(msg)   => write!(f, "bad address: {msg}"),
            Self::Send(msg)         => write!(f, "UDP send failed: {msg}"),
        }
    }
}

/// Send commands to robots over UDP.
#[async_trait]
pub trait CommandGateway: Send + Sync {
    /// Encode and send `command` to the robot identified by `target`.
    async fn send(&self, target: &RobotId, command: RobotCommand) -> Result<(), GatewayError>;
}
