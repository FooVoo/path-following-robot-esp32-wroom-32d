//! Fleet management server — composition root.
//!
//! Call [`run`] from `main` to start the full server with UDP ingress,
//! HTTP API, optional Postgres persistence, and SSE fan-out.

pub mod application;
pub mod domain;
pub mod http;
pub mod infrastructure;
pub mod ports;

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

use tracing::{info, warn};

use crate::config::{WIFI_CMD_PORT, WIFI_TEL_PORT};

use self::{
    application::{CommandService, FleetQueryService, IngestTelemetryService},
    domain::{RobotId, RobotSnapshot},
    http::{build_router, state::AppState},
    infrastructure::{
        BroadcastEventPublisher, InMemoryFleetRepository, PostgresFleetRepository,
        UdpCommandGateway, UdpTelemetryIngress,
    },
    ports::FleetRepository,
};

/// Server configuration — read from environment variables.
pub struct ServerConfig {
    pub host_ip:           Ipv4Addr,
    pub http_port:         u16,
    pub telemetry_udp_port: u16,
    pub cmd_udp_port:      u16,
    pub initial_robot_id:  Option<RobotId>,
    pub database_url:      Option<String>,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            host_ip: std::env::var("HOST_IP")
                .unwrap_or_else(|_| "0.0.0.0".into())
                .parse()
                .expect("HOST_IP must be a valid IPv4 address"),
            http_port: std::env::var("HTTP_PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()
                .expect("HTTP_PORT must be a valid port number"),
            telemetry_udp_port: std::env::var("TELEMETRY_UDP_PORT")
                .unwrap_or_else(|_| WIFI_TEL_PORT.to_string())
                .parse()
                .expect("TELEMETRY_UDP_PORT must be a valid port number"),
            cmd_udp_port: std::env::var("CMD_UDP_PORT")
                .unwrap_or_else(|_| WIFI_CMD_PORT.to_string())
                .parse()
                .expect("CMD_UDP_PORT must be a valid port number"),
            initial_robot_id: std::env::var("ROBOT_IP").ok().map(RobotId::new),
            database_url: std::env::var("DATABASE_URL").ok(),
        }
    }
}

/// Build all services, start background tasks, and serve HTTP until the
/// process exits.
pub async fn run(cfg: ServerConfig) {
    info!(
        "telemetry-server starting\n  HTTP  : {}:{}\n  UDP rx: :{} (telemetry)\n  UDP tx: :{} (commands)",
        cfg.host_ip, cfg.http_port, cfg.telemetry_udp_port, cfg.cmd_udp_port,
    );

    // ── Infrastructure ────────────────────────────────────────────────────────
    let mem_repo = Arc::new(InMemoryFleetRepository::new());
    let events   = Arc::new(BroadcastEventPublisher::new(64));
    let cmd_gw   = Arc::new(UdpCommandGateway::new(cfg.cmd_udp_port));

    let pg_repo: Option<Arc<PostgresFleetRepository>> = match cfg.database_url {
        Some(ref url) => match PostgresFleetRepository::connect(url).await {
            Ok(repo) => {
                let repo = Arc::new(repo);
                // Background cleanup: prune expired log rows every hour.
                repo.clone().start_cleanup_task();
                Some(repo)
            }
            Err(e) => {
                warn!("DB connection failed ({e}); telemetry logging disabled");
                None
            }
        },
        None => {
            info!("DATABASE_URL not set; telemetry logging disabled");
            None
        }
    };

    // ── Application services ──────────────────────────────────────────────────
    let ingest_svc = Arc::new(IngestTelemetryService::new(
        Arc::clone(&mem_repo),
        Arc::clone(&events),
    ));
    let query_svc = Arc::new(FleetQueryService::new(Arc::clone(&mem_repo)));
    let cmd_svc   = Arc::new(CommandService::new(Arc::clone(&mem_repo), Arc::clone(&cmd_gw)));

    // ── Pre-seed known robot (from ROBOT_IP env) ──────────────────────────────
    if let Some(ref id) = cfg.initial_robot_id {
        let snap = RobotSnapshot::placeholder(id.clone(), chrono::Utc::now());
        let _ = mem_repo.save_snapshot(&snap).await;
        info!("Robot seeded from ROBOT_IP env: {id}");
    }

    // ── Background: UDP telemetry ingress ─────────────────────────────────────
    let ingress = UdpTelemetryIngress::new(
        cfg.telemetry_udp_port,
        Arc::clone(&ingest_svc),
        pg_repo.clone(),
    );
    tokio::spawn(ingress.run());

    // ── HTTP server ───────────────────────────────────────────────────────────
    let state = AppState { query_svc, cmd_svc, events, pg_repo, mem_repo };
    let app   = build_router(state);

    let bind_addr = SocketAddr::new(IpAddr::V4(cfg.host_ip), cfg.http_port);
    let listener  = tokio::net::TcpListener::bind(bind_addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind HTTP to {bind_addr}: {e}"));

    info!("HTTP server ready → http://{bind_addr}");
    axum::serve(listener, app).await.expect("HTTP server crashed");
}

#[cfg(test)]
mod tests {
    //! Integration tests: spin up the router with an in-memory repo and fire
    //! HTTP requests using `axum::Router::oneshot`.

    use std::sync::Arc;

    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt as _; // for `.oneshot()`

    use crate::server::{
        application::{CommandService, FleetQueryService, IngestTelemetryService},
        domain::{RobotId, RobotSnapshot, TelemetryFrame},
        http::{build_router, state::AppState},
        infrastructure::{
            BroadcastEventPublisher, InMemoryFleetRepository, UdpCommandGateway,
        },
        ports::FleetRepository,
    };

    fn test_state() -> AppState {
        let mem_repo = Arc::new(InMemoryFleetRepository::new());
        let events   = Arc::new(BroadcastEventPublisher::new(8));
        let cmd_gw   = Arc::new(UdpCommandGateway::new(9000));

        AppState {
            query_svc: Arc::new(FleetQueryService::new(Arc::clone(&mem_repo))),
            cmd_svc:   Arc::new(CommandService::new(Arc::clone(&mem_repo), Arc::clone(&cmd_gw))),
            events,
            pg_repo:  None,
            mem_repo,
        }
    }

    async fn seed_robot(state: &AppState, ip: &str) {
        let frame = TelemetryFrame {
            state:          "IDLE".into(),
            lidar_left_cm:  None,
            lidar_right_cm: None,
            throttle_left:  0,
            throttle_right: 0,
            uptime_ms:      1_000,
            robot_id:       RobotId::new(ip),
        };
        let snap = RobotSnapshot::new(frame, chrono::Utc::now());
        state.mem_repo.save_snapshot(&snap).await.unwrap();
    }

    #[tokio::test]
    async fn health_returns_200() {
        let state = test_state();
        let app   = build_router(state);

        let resp = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn list_robots_initially_empty() {
        let state = test_state();
        let app   = build_router(state);

        let resp = app
            .oneshot(Request::builder().uri("/robots").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_robots_returns_seeded_robot() {
        let state = test_state();
        seed_robot(&state, "10.0.0.1").await;
        let app = build_router(state);

        let resp = app
            .oneshot(Request::builder().uri("/robots").body(Body::empty()).unwrap())
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "10.0.0.1");
    }

    #[tokio::test]
    async fn telemetry_503_when_empty() {
        let state = test_state();
        let app   = build_router(state);

        let resp = app
            .oneshot(Request::builder().uri("/telemetry").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn telemetry_200_after_seed() {
        let state = test_state();
        seed_robot(&state, "10.0.0.2").await;
        let app = build_router(state);

        let resp = app
            .oneshot(Request::builder().uri("/telemetry").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn command_button_503_when_no_robot() {
        let state = test_state();
        let app   = build_router(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/command/button")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn robot_logs_returns_empty_for_unknown() {
        let state = test_state();
        let app   = build_router(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/robots/9.9.9.9/logs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 0);
        assert!(json["logs"].as_array().unwrap().is_empty());
    }
}
