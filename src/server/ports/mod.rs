//! Port traits — outbound interfaces the application layer depends on.
//!
//! All infrastructure implementations live in `crate::server::infrastructure`.

pub mod command_gateway;
pub mod event_publisher;
pub mod fleet_repository;

pub use command_gateway::CommandGateway;
pub use event_publisher::EventPublisher;
pub use fleet_repository::FleetRepository;
