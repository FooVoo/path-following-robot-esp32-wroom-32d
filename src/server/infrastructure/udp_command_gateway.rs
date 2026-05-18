//! `UdpCommandGateway` — sends 4-byte command packets to robots over UDP.

use async_trait::async_trait;
use tracing::info;

use crate::server::{
    domain::{RobotCommand, RobotId},
    ports::command_gateway::{CommandGateway, GatewayError},
};

pub struct UdpCommandGateway {
    cmd_port: u16,
}

impl UdpCommandGateway {
    pub fn new(cmd_port: u16) -> Self {
        Self { cmd_port }
    }
}

#[async_trait]
impl CommandGateway for UdpCommandGateway {
    async fn send(&self, target: &RobotId, command: RobotCommand) -> Result<(), GatewayError> {
        let addr_str = format!("{}:{}", target.as_str(), self.cmd_port);
        let robot_addr: std::net::SocketAddr = addr_str
            .parse()
            .map_err(|e| GatewayError::BadAddress(format!("{e}")))?;

        let sock = tokio::net::UdpSocket::bind("0.0.0.0:0")
            .await
            .map_err(|e| GatewayError::Send(format!("bind failed: {e}")))?;

        let packet = command.to_wire();
        sock.send_to(&packet, robot_addr)
            .await
            .map_err(|e| GatewayError::Send(format!("{e}")))?;

        info!("CMD {:?} sent to {robot_addr}", &packet[..2]);
        Ok(())
    }
}
