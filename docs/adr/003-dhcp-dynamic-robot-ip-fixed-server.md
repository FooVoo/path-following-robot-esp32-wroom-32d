# ADR-003 — DHCP for Robot IP; Fixed-Endpoint Axum Fleet Server

| Field    | Value                                      |
|----------|--------------------------------------------|
| Date     | 2026-05-18                                 |
| Status   | **Accepted**                               |
| Deciders | FooVoo                                     |
| Supersedes | Static-IP WiFi configuration (pre ADR-003) |

---

## Context

The original WiFi implementation assigned robots a **static IPv4 address** at
compile time via constants in `src/config.rs`:

```rust
pub const WIFI_STATIC_IP:     [u8;4] = [192, 168, 1, 200];
pub const WIFI_SUBNET_PREFIX: u8     = 24;
pub const WIFI_GATEWAY:       [u8;4] = [192, 168, 1,   1];
pub const WIFI_BROADCAST_IP:  [u8;4] = [192, 168, 1, 255];
```

And the `smoltcp` interface was initialised with a hard-coded `IpCidr`:

```rust
iface.update_ip_addrs(|a| {
    a.push(IpCidr::new(
        IpAddress::v4(192, 168, 1, 200),
        WIFI_SUBNET_PREFIX,
    )).ok();
});
```

This approach has several problems that became apparent once fleet management
was introduced:

### Problems with static IP

| Problem | Detail |
|---|---|
| **One binary = one IP** | Flashing two robots from the same build results in an IP conflict; every unit needs a custom build or manual config edit. |
| **Router dependency** | The IP must be manually reserved outside the DHCP pool on every new network. |
| **Compile-time coupling** | Network topology leaks into the firmware image.  Moving to a different subnet requires a reflash. |
| **No self-identification** | The host server cannot tell which robot sent a given telemetry frame when robots only communicate via the same broadcast. |
| **Fleet management impossible** | A fleet UI needs stable identifiers for each robot.  With static IPs this requires per-unit build coordination. |

### Server placement

The fleet management server (Axum / `telemetry-server` binary) runs on a host
machine with a **stable, operator-chosen address**.  Robots do not connect to
the server — they broadcast to `255.255.255.255`.  The server always knows the
source IP from the UDP sender address (and now from the frame payload).

---

## Decision

### Robot side — DHCP

**Remove all static-IP constants.  Have the robot obtain its IP from the DHCP
server on the LAN.**

```rust
// src/adapters/esp32/wifi.rs — simplified
let assigned_ip: [u8; 4] = block_on(async {
    // ... smoltcp DHCP exchange via dhcpv4::Socket ...
    // Returns the IP assigned by the router
});
```

The assigned IP is stored in `WifiAdapterInner::assigned_ip` and injected into
every telemetry JSON frame as the `"ip"` field:

```json
{"s":"PLAY","ll":125,"lr":98,"tl":50,"tr":50,"ms":12345,"ip":"192.168.1.42"}
```

All other DHCP parameters (subnet mask, gateway, DNS) are accepted from the
server and applied to the smoltcp interface automatically — no constants needed.

A single timeout constant is retained:

```rust
/// Maximum ms the robot waits for a DHCP lease before falling back to
/// offline (no telemetry, no remote control) mode.
pub const WIFI_DHCP_TIMEOUT_MS: u64 = 15_000;
```

### Server side — fixed well-known endpoint

The Axum fleet server binds to a **configurable, operator-chosen address and
port** (default `0.0.0.0:8080`).  It accepts telemetry from any robot without
prior registration.  Robots are identified by the `"ip"` field in the frame;
if absent, the UDP sender address is used as fallback.

```
Configuration (environment variables):
  HOST_IP            — bind address (default 0.0.0.0)
  HTTP_PORT          — HTTP UI + API port (default 8080)
  TELEMETRY_UDP_PORT — UDP listen port for robot frames (default 9001)
  CMD_UDP_PORT       — UDP source port for sending commands (default 9000)
  DATABASE_URL       — optional PostgreSQL URL for telemetry log storage
```

The server's IP is **not baked into the robot firmware** in any way.  The robot
only needs to know the LAN broadcast address (`255.255.255.255`), which is
universal and requires no per-deployment configuration.

Commands are sent **unicast** to the IP extracted from the latest telemetry
frame for that robot (or the `ROBOT_IP` env var if no frame has been received
yet):

```
POST /command/{ip}/button
POST /command/{ip}/throttle
```

---

## Consequences

**Positive**

- **Zero network config per robot** — flash once, deploy anywhere.  The router
  assigns the IP; the robot self-reports it.
- **Unlimited fleet size** — any number of robots can join transparently.  Each
  is identified by its DHCP-assigned IP in the telemetry frame.
- **Stable server endpoint** — the server is at a known, human-chosen address.
  Only the server's address needs to be known by the operator; robots do not
  need to know it.
- **No IP conflicts** — DHCP ensures unique addresses even if all units are
  flashed from the same binary.
- **IP changes are self-healing** — if the router reassigns a different IP on
  reconnect, the next telemetry frame updates the server's in-memory record
  automatically.

**Negative / trade-offs**

- **Commands require prior telemetry** — the server learns a robot's current
  IP from incoming frames.  If a robot has never sent a frame (or the server
  restarted), `POST /command` for that robot fails with a 503 until the first
  frame arrives.  The `ROBOT_IP` env var is provided as a static fallback for
  single-robot deployments.
- **IP instability** — DHCP leases can expire.  If a robot's IP changes mid-
  session, the server treats it as a new robot entry.  MAC-based DHCP
  reservations in the router are recommended for persistent identities.
- **Broadcast dependency** — telemetry uses `255.255.255.255` (limited
  broadcast), which does not cross layer-3 boundaries (routers, VLANs).  The
  robot and the server must be on the same subnet.  If Docker is used, the
  container must either expose UDP port 9001 (unicast only) or use
  `network_mode: host` (Linux only) for broadcast support.

---

## Alternatives considered

### A: Static IP per robot (pre-ADR-003 state)

Works for a single-robot demo.  Requires a unique build per unit and manual
network planning.  Rejected as a long-term approach — see "Problems" above.

### B: Static IP + per-robot build flag (`features = ["robot-1"]`)

Could encode the IP in a Cargo feature.  Avoids reflashing the same binary
twice but still requires a separate build per unit and manual IP reservation
on every new network.  Adds feature-matrix complexity to `Cargo.toml`.
Rejected — the complexity cost exceeds the benefit over DHCP.

### C: mDNS / Bonjour for server discovery

Would let robots discover the server by hostname instead of broadcasting.
`smoltcp 0.12` does not include an mDNS implementation.  Adding one would
require either an external crate (not available in `no_std` + esp-wifi
context at the time of writing) or a hand-rolled implementation.  The
broadcast approach is simpler and sufficient for single-LAN deployments.
Rejected for complexity.

### D: MQTT broker

A broker-based architecture (robot → MQTT → server) would give reliable
delivery and natural topic-based fleet organisation.  However, MQTT requires
TCP, and TCP connection management in the cooperative `block_on` tick model
adds significant latency risk and complexity.  MQTT client crates are also
not available for `no_std` / smoltcp.  Rejected — out of scope for the
current hardware constraints.

### E: Server IP hard-coded in firmware

The robot could send telemetry directly to the server's unicast IP instead
of broadcasting.  This would eliminate the broadcast dependency but would
require re-flashing every time the server moves to a different machine.
Rejected — the broadcast approach is more portable and requires no server
IP knowledge in the firmware.
