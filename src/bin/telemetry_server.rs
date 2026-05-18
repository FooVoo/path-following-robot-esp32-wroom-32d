//! Fleet-management server entry point.
//!
//! All business logic lives in `path_following_robot::server`.
//! This file is intentionally minimal — configuration is read from the
//! environment and forwarded to `server::run`.
//!
//! ## Environment variables
//!
//! | Variable             | Default               | Description                                    |
//! |----------------------|-----------------------|------------------------------------------------|
//! | `HOST_IP`            | `0.0.0.0`             | IP the HTTP server binds to                    |
//! | `HTTP_PORT`          | `8080`                | TCP port for the HTTP server                   |
//! | `TELEMETRY_UDP_PORT` | *(from config crate)* | UDP port to receive robot telemetry            |
//! | `CMD_UDP_PORT`       | *(from config crate)* | UDP port for sending commands to robots        |
//! | `ROBOT_IP`           | *(none)*              | Pre-seed a robot IP before first frame arrives |
//! | `DATABASE_URL`       | *(none)*              | PostgreSQL URL; absent → log storage disabled  |
//!
//! ## Build and run
//!
//! ```bash
//! cargo run --features host-server --bin telemetry-server
//!
//! # With Postgres:
//! DATABASE_URL=postgres://user:pass@localhost/robots \
//!   cargo run --features host-server --bin telemetry-server
//! ```

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = path_following_robot::server::ServerConfig::from_env();
    path_following_robot::server::run(cfg).await;
}
