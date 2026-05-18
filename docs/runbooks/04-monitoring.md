# Runbook 04 — Monitoring

> **Audience:** Operators monitoring robot health during farm operations.

---

## 1  Serial log (UART0)

Connect a USB cable to the ESP32.  Open the serial monitor at **115 200 baud**.

```bash
espflash monitor --port /dev/cu.usbserial-0001
# or
screen /dev/ttyUSB0 115200
```

### Log prefix legend

```
I (ms) module: message    ← INFO
D (ms) module: message    ← DEBUG
W (ms) module: message    ← WARN
E (ms) module: message    ← ERROR
```

`(ms)` is the milliseconds since boot reported by `esp-hal`'s timer.

---

## 2  Normal boot sequence

```
I (321)  path_following_robot: WiFi connecting to "FarmNet"...
I (4231) path_following_robot: WiFi: connected — IP 192.168.1.42 — remote control + telemetry enabled
I (4232) path_following_robot: State: IDLE
```

The IP shown is **DHCP-assigned** and may differ between boots.  The assigned
address is embedded in every telemetry frame (`"ip"` field) so the fleet server
and any monitoring script can always identify the robot.

The robot is ready when the `State: IDLE` line appears.

---

## 3  Telemetry fields

Every telemetry datagram is a UTF-8 JSON object on UDP port 9001:

```json
{"s":"PLAY","ll":125,"lr":98,"tl":50,"tr":50,"ms":12345,"ip":"192.168.1.42"}
```

| Field | Description                                             |
|-------|---------------------------------------------------------|
| `s`   | State: `IDLE` `RECORD` `READY` `PLAY` `AVOIDING` `HALT` |
| `ll`  | Left LIDAR distance in centimetres; `-1` = stale        |
| `lr`  | Right LIDAR distance in centimetres; `-1` = stale       |
| `tl`  | Left motor throttle, range `[-100, 100]`                |
| `tr`  | Right motor throttle, range `[-100, 100]`               |
| `ms`  | Uptime in milliseconds since boot                       |
| `ip`  | DHCP-assigned IPv4 address of this robot                |

---

## 4  State transition log

Whenever the FSM changes state, an `INFO` line is emitted:

```
I (7820)  path_following_robot: State: IDLE → RECORD
I (12501) path_following_robot: State: RECORD → READY (34 commands)
I (13042) path_following_robot: State: READY → PLAY
I (15110) path_following_robot: State: PLAY → AVOIDING (left obstacle 32 cm)
I (16340) path_following_robot: State: AVOIDING → PLAY
I (21600) path_following_robot: State: PLAY → HALT (path complete)
```

---

## 5  LIDAR health indicators

| Condition | Serial output | Telemetry `ll`/`lr` | Meaning |
|---|---|---|---|
| Normal reading | `D (...) lidar_l: 120 cm` | positive integer | Sensor OK |
| Stale (no new frames) | `W (...) lidar_l: stale` | `-1` | No UART data for `STALE_TICKS` ticks |
| Checksum error | `W (...) lidar_l: bad checksum` | previous value | Corrupted frame — single event is OK |
| Sensor disconnected | `W (...) lidar_l: stale` repeatedly | `-1` continuously | Check UART wiring |

---

## 6  Obstacle avoidance monitoring

During `AVOIDING`, the serial log shows:

```
I (15110) path_following_robot: State: PLAY → AVOIDING (left obstacle 32 cm)
D (15120) path_following_robot: avoid — phase BACK (t=0 ms)
D (15220) path_following_robot: avoid — phase TURN (t=100 ms)
D (15420) path_following_robot: avoid — phase CLEAR (left=68 cm, right=210 cm)
I (15430) path_following_robot: State: AVOIDING → PLAY
```

If `AVOIDING` transitions to `HALT` instead:

```
W (20000) path_following_robot: avoidance timeout — halting
I (20001) path_following_robot: State: AVOIDING → HALT
```

This means the obstacle was not cleared within the avoidance timeout window
(`AVOIDANCE_TIMEOUT_MS` in `config.rs`, default 5000 ms).

---

## 7  WiFi health

### Normal WiFi operation

```
I (4231) path_following_robot: WiFi: connected — IP 192.168.1.42 — remote control + telemetry enabled
D (4400) path_following_robot: telemetry sent (12 bytes)
D (4600) path_following_robot: command received: throttle 50/50
```

### WiFi reconnection

`esp-wifi` currently does not auto-reconnect after a dropped association.  If
the AP reboots while the robot is operating:

```
W (...) path_following_robot: WiFi socket error — retrying
```

The robot will continue operating locally.  To restore WiFi, power-cycle the
robot or wait for a planned firmware update that adds reconnection logic.

---

## 8  Key configuration values to know

| Constant | Default | Effect |
|---|---|---|
| `OBSTACLE_DISTANCE_CM` | 50 | LIDARs closer than this trigger `AVOIDING` |
| `STALE_TICKS` | 10 | Ticks without LIDAR data before reading is discarded |
| `AVOIDANCE_TIMEOUT_MS` | 5000 | Max time in `AVOIDING` before `HALT` |
| `TELEMETRY_INTERVAL_MS` | 200 | ms between UDP telemetry datagrams |
| `PATH_CAPACITY` | 512 | Max recorded path commands |

These are all in `src/config.rs` and require a rebuild to change.
