//! `UdpTelemetryIngress` — background task that receives robot UDP frames,
//! calls `IngestTelemetryService`, and optionally persists to Postgres.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

use tracing::{info, warn};

use crate::server::{
    application::IngestTelemetryService,
    domain::TelemetryFrame,
    infrastructure::{
        BroadcastEventPublisher, InMemoryFleetRepository, PostgresFleetRepository,
    },
    ports::FleetRepository,
};

pub struct UdpTelemetryIngress {
    port:    u16,
    ingest:  Arc<IngestTelemetryService<InMemoryFleetRepository, BroadcastEventPublisher>>,
    pg_repo: Option<Arc<PostgresFleetRepository>>,
}

impl UdpTelemetryIngress {
    pub fn new(
        port:    u16,
        ingest:  Arc<IngestTelemetryService<InMemoryFleetRepository, BroadcastEventPublisher>>,
        pg_repo: Option<Arc<PostgresFleetRepository>>,
    ) -> Self {
        Self { port, ingest, pg_repo }
    }

    /// Run the ingress loop forever.  Intended to be `tokio::spawn`-ed.
    pub async fn run(self) {
        let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), self.port);
        let sock = tokio::net::UdpSocket::bind(bind)
            .await
            .unwrap_or_else(|e| panic!("UDP bind failed on {bind}: {e}"));
        info!("UDP telemetry ingress listening on {bind}");

        let mut buf = [0u8; 512];
        loop {
            let Ok((len, src)) = sock.recv_from(&mut buf).await else {
                continue;
            };

            let raw = &buf[..len];
            let fallback_ip = src.ip().to_string();

            let frame = match TelemetryFrame::decode(raw, &fallback_ip) {
                Ok(f)  => f,
                Err(e) => {
                    warn!("decode error from {src}: {e}");
                    continue;
                }
            };

            // Persist to Postgres (non-blocking, non-fatal).
            if let Some(ref pg) = self.pg_repo {
                let pg2    = Arc::clone(pg);
                let id     = frame.robot_id.clone();
                let frame2 = frame.clone();
                tokio::spawn(async move {
                    if let Err(e) = pg2.insert_log(&id, &frame2).await {
                        warn!("pg log insert failed: {e:?}");
                    }
                    if let Err(e) = pg2.save_snapshot(&crate::server::domain::RobotSnapshot::new(
                        frame2.clone(),
                        chrono::Utc::now(),
                    )).await {
                        warn!("pg snapshot upsert failed: {e:?}");
                    }
                });
            }

            // Update in-memory state + publish SSE event.
            self.ingest.ingest(frame).await;
        }
    }
}
