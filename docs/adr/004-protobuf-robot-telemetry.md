# ADR-004 — Protocol Buffers for Robot→Server Telemetry

| Field      | Value                            |
|------------|----------------------------------|
| Date       | 2026-05-18                       |
| Status     | **Accepted**                     |
| Deciders   | FooVoo                           |
| Supersedes | JSON UDP telemetry (pre ADR-004) |

---

## Context

The telemetry server receives real-time sensor frames from robots over UDP.
Each frame carries: robot ID, motor state, two lidar distances, two throttle
values, and an uptime counter.

The original implementation serialised these frames as JSON
(`{"id":"…","s":"PLAY","ll":42,…}`), chosen for its debuggability and zero
build-time tooling.

Three pressures drove a re-evaluation:

1. **Payload size.** A typical JSON frame is ~90–120 bytes; the same data in
   proto binary is ~25–35 bytes — a 60–70 % reduction.  On a lossy 2.4 GHz
   mesh with MTU around 250 bytes this headroom matters.

2. **Schema drift.** Renaming a JSON field silently breaks old consumers.
   Proto's numbered field tags allow additive evolution without coordination.

3. **ESP32 encoding cost.** `serde_json` on `no_std` requires `heapless`
   workarounds and spends cycles on UTF-8 escaping.  `prost` encodes directly
   into a fixed-size stack buffer with no allocator involvement.

---

## Decision

**Replace JSON with Protocol Buffers (proto3) for the robot→server UDP
telemetry channel.**

- One canonical schema lives at `proto/telemetry.proto`.
- The Rust types are generated at build time by `prost-build` (see
  `build.rs`).
- The robot firmware encodes frames with `prost` (`no_std`,
  `default-features = false, features = ["prost-derive"]`).
- The server decodes frames with `prost` (`std` activated by the
  `host-server` Cargo feature).

### Wire format

```
proto/telemetry.proto

message TelemetryFrame {
  string robot_id        = 1;
  sint32 lidar_left_cm   = 2;
  sint32 lidar_right_cm  = 3;
  sint32 throttle_left   = 4;
  sint32 throttle_right  = 5;
  uint64 uptime_ms       = 6;
  string state           = 7;
}
```

`sint32` is used for signed sensor values (varint zig-zag encoding keeps
small negative numbers compact).

### Backwards compatibility shim

The server's `TelemetryFrame::decode()` performs a first-byte dispatch:

- `0x7B` (`{`) → legacy JSON path via `from_json()`
- anything else → proto path via `from_proto()`

This lets old robots (still sending JSON) coexist with proto robots during a
rolling firmware upgrade.  The shim is intentionally temporary; once all
robots are flashed the JSON path will be removed.

### No change to UI/REST

The HTTP endpoints served by the fleet UI continue to use JSON (via `axum`
+ `serde_json`).  Protobuf is confined to the internal robot→server UDP
channel only.

---

## Alternatives Considered

### Keep JSON

- ✅ No tooling, trivially debuggable with `tcpdump`
- ❌ ~4× larger payloads
- ❌ Schema drift risk; `serde` rename requires code + firmware sync

### MessagePack

- ✅ Smaller than JSON, no schema required
- ✅ `rmp-serde` works on `no_std`
- ❌ Still untyped; field identity by position, not numbered tag
- ❌ No code-gen; manual struct alignment between firmware and server

### FlatBuffers / Cap'n Proto

- ✅ Zero-copy reads
- ❌ No mature `no_std` Rust crates at time of decision
- ❌ Higher complexity for a 7-field message

### Custom binary struct

- ✅ Minimum bytes, no dependencies
- ❌ Brittle; any field reorder breaks all consumers
- ❌ Manual versioning

---

## Consequences

**Positive**
- Frame size reduced ~65 % (90+ bytes JSON → ~30 bytes proto).
- Schema is the single source of truth; generated types are used in both
  firmware and server, eliminating accidental field-name drift.
- Additive changes (new sensor field) require only a new proto tag; old
  servers silently ignore unknown fields.
- `prost` encodes into a `[u8; N]` stack buffer on the ESP32 — no heap
  allocation required.

**Negative / trade-offs**
- Proto frames are not human-readable; a `protoc`-based decode step is
  needed to inspect raw UDP traffic.
- `protoc` must be present on the build host (`brew install protobuf`).
- Firmware and server proto definitions must stay in sync; a schema mismatch
  produces silent partial decodes (unknown fields are dropped, not errored).

**Migration path**
1. Server already deployed with the first-byte shim (see above).
2. Robots flashed with new firmware begin sending proto frames automatically.
3. Once all robots are upgraded, remove the `from_json()` fallback and the
   `0x7B` branch in `TelemetryFrame::decode()`.
