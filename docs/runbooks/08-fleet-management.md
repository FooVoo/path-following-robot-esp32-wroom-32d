# Runbook 08 — Fleet Management Server

> **Audience:** Operators running the `telemetry-server` binary to monitor and
> control one or more robots from a web UI.

---

## 1  Overview

`telemetry-server` is a host-side binary (compiled with `--features host-server`)
that:

- Listens for UDP telemetry broadcasts from any robot on the LAN (port `9001`).
- Serves a web UI at `http://<host>:8080/` with a live fleet status table.
- Streams live updates to the browser via **Server-Sent Events** (SSE).
- Stores every telemetry frame in **PostgreSQL** with a 2-day TTL (optional).
- Exposes a REST API for per-robot log pagination and remote control.

---

## 2  Quick start (Docker — recommended)

```bash
# Clone and enter the project root
cd path-following-robot-esp32-wroom-32d

# Start Postgres + telemetry-server
docker compose up --build

# Open the fleet UI
open http://localhost:8080
```

Stop and keep data:

```bash
docker compose down
```

Stop and wipe all telemetry data:

```bash
docker compose down -v
```

---

## 3  Quick start (binary, no Docker)

```bash
# Build
cargo build --release --features host-server --bin telemetry-server

# Run without Postgres (in-memory only — data lost on restart)
./target/release/telemetry-server

# Run with Postgres (must be running separately)
DATABASE_URL=postgres://robots:robots@localhost/robots \
  ./target/release/telemetry-server
```

---

## 4  Configuration — environment variables

| Variable             | Default    | Description |
|----------------------|------------|-------------|
| `HOST_IP`            | `0.0.0.0`  | IP address the HTTP server binds to |
| `HTTP_PORT`          | `8080`     | TCP port for the HTTP server |
| `TELEMETRY_UDP_PORT` | `9001`     | UDP port to receive robot telemetry frames |
| `CMD_UDP_PORT`       | `9000`     | UDP port for sending remote-control commands |
| `ROBOT_IP`           | *(none)*   | Seed a known robot IP before the first telemetry frame arrives |
| `DATABASE_URL`       | *(none)*   | PostgreSQL URL; if absent, log storage is disabled |
| `RUST_LOG`           | `info`     | Log filter (`info`, `debug`, `trace`) |

### Docker Compose defaults

When using `docker compose up`, the `DATABASE_URL` is automatically set to:

```
postgres://robots:robots@postgres:5432/robots
```

---

## 5  Network topology

```
LAN  192.168.1.0/24
┌─────────────────────────────────────────────────────────────────┐
│                                                                 │
│   [WiFi AP]  192.168.1.1                                        │
│       │                                                         │
│       │ WiFi 2.4 GHz                                            │
│       ├──── Robot A  192.168.1.x  (DHCP)                        │
│       │       │  telemetry UDP broadcast → 255.255.255.255:9001 │
│       │       └  commands ← UDP unicast ← :9000                 │
│       │                                                         │
│       ├──── Robot B  192.168.1.y  (DHCP)                        │
│       │       │  (same ports)                                   │
│       │                                                         │
│       └──── [Host PC]  192.168.1.z                              │
│               │  telemetry-server  UDP :9001 (listen)           │
│               │                   TCP :8080  (HTTP + SSE)       │
│               └─ browser  http://localhost:8080/                │
└─────────────────────────────────────────────────────────────────┘
```

> **Broadcast and Docker:** limited-broadcast (`255.255.255.255`) does not
> cross Docker's bridge network.  For a real robot on the LAN, either:
>
> - **Option A (recommended):** have the robot send unicast to the host IP.
>   Works with the default bridge networking in `docker-compose.yml`.
> - **Option B (Linux only):** uncomment `network_mode: host` in
>   `docker-compose.yml` and change `DATABASE_URL` to use `127.0.0.1`.

---

## 6  Web UI

### Fleet overview — `GET /`

A dark-themed table refreshed live via SSE.  One row per robot.

| Column | Meaning |
|---|---|
| Status | Green dot = frame received ≤ 10 s ago; grey = offline |
| IP | Robot's DHCP-assigned address (links to robot detail page) |
| State | Current FSM state (`IDLE`, `RECORD`, `PLAY`, etc.) |
| LIDAR L / R | Left and right distances in cm; `−1` = stale |
| Throttle L / R | Current motor throttle `[−100, 100]` |
| Frames | Total telemetry frames received since server start |
| Last seen | Time since last frame |

### Robot detail — `GET /robots/{ip}`

- Live state panel updated by SSE (same feed, filtered by `robot_ip`).
- Paginated log table loaded from the database (most recent 100 rows first).
- **Load older** button pages back through the 2-day history.
- If PostgreSQL is not configured the log table shows *"database not available"*.

---

## 7  REST API

| Method | Path | Description |
|---|---|---|
| `GET` | `/health` | Returns `{"status":"ok"}` |
| `GET` | `/robots` | JSON array of all known robots with their latest snapshot |
| `GET` | `/robots/{ip}` | HTML robot detail page |
| `GET` | `/robots/{ip}/logs` | JSON log records; query params: `limit` (default 100), `offset` (default 0) |
| `GET` | `/events` | SSE stream; one event per telemetry frame received |
| `POST` | `/telemetry` | Accept a raw JSON telemetry frame (testing / simulation) |
| `POST` | `/command/button` | Send button press to most-recently-seen robot |
| `POST` | `/command/throttle` | Send throttle to most-recently-seen robot; body `{"left":N,"right":N}` |
| `POST` | `/command/{ip}/button` | Send button press to a specific robot by IP |
| `POST` | `/command/{ip}/throttle` | Send throttle to a specific robot by IP |

### `/robots/{ip}/logs` response

```json
{
  "total": 8640,
  "logs": [
    {
      "id": 8640,
      "received_at": "2026-05-18T17:00:00Z",
      "frame": {"s":"PLAY","ll":125,"lr":98,"tl":50,"tr":50,"ms":12345,"ip":"192.168.1.42"}
    }
  ]
}
```

### SSE event format

```
data: {"robot_ip":"192.168.1.42","frame":{...},"timestamp":"2026-05-18T17:00:00Z"}
```

JavaScript example:

```js
const es = new EventSource('/events');
es.onmessage = ({ data }) => {
    const { robot_ip, frame } = JSON.parse(data);
    console.log(robot_ip, frame.s);
};
```

---

## 8  PostgreSQL schema

```sql
-- Summary row per robot (upserted on every frame)
CREATE TABLE robots (
    ip            TEXT PRIMARY KEY,
    last_state    TEXT        NOT NULL,
    last_telemetry JSONB      NOT NULL,
    last_seen_at  TIMESTAMPTZ NOT NULL,
    total_frames  BIGINT      NOT NULL DEFAULT 0
);

-- Append-only telemetry log with 2-day TTL
CREATE TABLE telemetry_logs (
    id         BIGSERIAL    PRIMARY KEY,
    robot_ip   TEXT         NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL,   -- NOW() + INTERVAL '2 days'
    frame       JSONB        NOT NULL
);

CREATE INDEX ON telemetry_logs (robot_ip, received_at DESC);
CREATE INDEX ON telemetry_logs (expires_at);
```

Rows older than `expires_at` are deleted by an hourly background task.
The cleanup runs automatically inside the server process — no cron job needed.

---

## 9  Running without PostgreSQL

If `DATABASE_URL` is not set the server starts in **in-memory mode**:

- Fleet table and SSE work normally.
- `GET /robots/{ip}/logs` returns `503 {"error":"database not configured"}`.
- All telemetry data is lost when the server restarts.

---

## 10  Single-robot quick-command (Python)

```python
import requests

SERVER = "http://localhost:8080"

# Synthetic button press (start/stop recording)
requests.post(f"{SERVER}/command/button")

# Drive forward at 50%
requests.post(f"{SERVER}/command/throttle", json={"left": 50, "right": 50})

# Target a specific robot by IP
requests.post(f"{SERVER}/command/192.168.1.42/throttle", json={"left": 30, "right": 30})
```
