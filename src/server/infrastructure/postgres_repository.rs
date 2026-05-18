//! `PostgresFleetRepository` — PostgreSQL-backed implementation of `FleetRepository`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use tracing::{info, warn};

use crate::server::{
    domain::{LogEntry, RobotId, RobotSnapshot, TelemetryFrame},
    ports::fleet_repository::{FleetRepository, RepoResult, RepositoryError},
};

pub struct PostgresFleetRepository {
    pool: PgPool,
}

impl PostgresFleetRepository {
    /// Connect and ensure the schema exists.
    pub async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPool::connect(url).await?;
        Self::migrate(&pool).await?;
        info!("PostgreSQL connected; schema verified");
        Ok(Self { pool })
    }

    async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS robots (
                ip             TEXT PRIMARY KEY,
                last_state     TEXT        NOT NULL,
                last_telemetry JSONB       NOT NULL,
                last_seen_at   TIMESTAMPTZ NOT NULL,
                total_frames   BIGINT      NOT NULL DEFAULT 0
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS telemetry_logs (
                id          BIGSERIAL   PRIMARY KEY,
                robot_ip    TEXT        NOT NULL,
                received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                expires_at  TIMESTAMPTZ NOT NULL,
                frame       JSONB       NOT NULL
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tl_robot_received
             ON telemetry_logs (robot_ip, received_at DESC)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tl_expires
             ON telemetry_logs (expires_at)",
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Spawn a background task that deletes expired log rows every hour.
    ///
    /// Takes `Arc<Self>` so the pool stays alive for the lifetime of the task.
    pub fn start_cleanup_task(self: std::sync::Arc<Self>) {
        tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval(std::time::Duration::from_secs(3_600));
            loop {
                ticker.tick().await;
                match sqlx::query(
                    "DELETE FROM telemetry_logs WHERE expires_at < NOW()",
                )
                .execute(&self.pool)
                .await
                {
                    Ok(r) => info!(
                        "Pruned {} expired telemetry log rows",
                        r.rows_affected()
                    ),
                    Err(e) => warn!("Log cleanup failed: {e}"),
                }
            }
        });
    }
}

#[async_trait]
impl FleetRepository for PostgresFleetRepository {
    async fn save_snapshot(&self, snap: &RobotSnapshot) -> RepoResult<()> {
        let frame_json = serde_json::to_value(&snap.latest)
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        sqlx::query(
            "INSERT INTO robots (ip, last_state, last_telemetry, last_seen_at, total_frames)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (ip) DO UPDATE SET
               last_state     = EXCLUDED.last_state,
               last_telemetry = EXCLUDED.last_telemetry,
               last_seen_at   = EXCLUDED.last_seen_at,
               total_frames   = EXCLUDED.total_frames",
        )
        .bind(snap.id.as_str())
        .bind(&snap.latest.state)
        .bind(&frame_json)
        .bind(snap.last_seen)
        .bind(snap.received_count as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(())
    }

    async fn get_snapshot(&self, _id: &RobotId) -> RepoResult<Option<RobotSnapshot>> {
        // The snapshot is always in-memory; this implementation is a no-op
        // for reads (the in-memory repository owns the read path).
        Ok(None)
    }

    async fn list_snapshots(&self) -> RepoResult<Vec<RobotSnapshot>> {
        Ok(vec![])
    }

    async fn most_recent(&self) -> RepoResult<Option<RobotId>> {
        Ok(None)
    }

    async fn insert_log(&self, id: &RobotId, frame: &TelemetryFrame) -> RepoResult<()> {
        let frame_json = serde_json::to_value(frame)
            .map_err(|e| RepositoryError::Database(e.to_string()))?;

        sqlx::query(
            "INSERT INTO telemetry_logs (robot_ip, frame, expires_at)
             VALUES ($1, $2, NOW() + INTERVAL '2 days')",
        )
        .bind(id.as_str())
        .bind(&frame_json)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(())
    }

    async fn count_logs(&self, id: &RobotId) -> RepoResult<i64> {
        let total: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM telemetry_logs WHERE robot_ip = $1")
                .bind(id.as_str())
                .fetch_one(&self.pool)
                .await
                .map_err(|e| RepositoryError::Database(e.to_string()))?;
        Ok(total)
    }

    async fn query_logs(
        &self,
        id: &RobotId,
        limit: i64,
        offset: i64,
    ) -> RepoResult<Vec<LogEntry>> {
        let rows = sqlx::query(
            "SELECT id, received_at, frame
             FROM telemetry_logs
             WHERE robot_ip = $1
             ORDER BY received_at DESC
             LIMIT $2 OFFSET $3",
        )
        .bind(id.as_str())
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let log_id: i64        = row.get("id");
            let received_at: DateTime<Utc> = row.get("received_at");
            let frame_val: serde_json::Value = row.get("frame");
            let frame: TelemetryFrame = serde_json::from_value(frame_val)
                .map_err(|e| RepositoryError::Database(e.to_string()))?;
            entries.push(LogEntry { id: log_id, received_at, frame });
        }
        Ok(entries)
    }
}
