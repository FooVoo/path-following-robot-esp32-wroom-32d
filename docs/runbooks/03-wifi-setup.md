# Runbook 03 — WiFi Setup

> **Audience:** Operators configuring the network environment.

---

## 1  Network topology

```
  Farm LAN  192.168.1.0/24
  ┌──────────────────────────────────────────────────────┐
  │                                                      │
  │   [WiFi AP / DHCP server]                            │
  │   192.168.1.1                                        │
  │       │ WiFi 2.4 GHz                                 │
  │       ├── [Robot A]  DHCP → e.g. 192.168.1.42        │
  │       │     telemetry broadcast → 255.255.255.255:9001│
  │       │     commands  ← unicast ← :9000              │
  │       │                                              │
  │       ├── [Robot B]  DHCP → e.g. 192.168.1.43        │
  │       │     (same ports)                             │
  │       │                                              │
  │       └── [Host PC]  192.168.1.x                     │
  │             telemetry-server UDP :9001               │
  │             fleet UI  TCP :8080                      │
  └──────────────────────────────────────────────────────┘
```

Robots obtain their IP via **DHCP** — no static address configuration is
required.  The assigned IP is embedded in every telemetry frame (`"ip"` field)
so the fleet server can identify individual robots automatically.  See
[ADR-003](../adr/003-dhcp-dynamic-robot-ip-fixed-server.md) for the rationale.

---

## 2  Required configuration in `src/config.rs`

Only two WiFi constants need to be changed before flashing:

```rust
pub const WIFI_SSID:     &str = "YourSSID";
pub const WIFI_PASSWORD: &str = "YourPassword";
```

All other network parameters are handled automatically:

| Constant | Default | How it is determined |
|---|---|---|
| `WIFI_CMD_PORT` | `9000` | Compile-time constant; robot listens here |
| `WIFI_TEL_PORT` | `9001` | Compile-time constant; robot broadcasts here |
| `TELEMETRY_INTERVAL_MS` | `200` ms | Compile-time constant |
| `WIFI_DHCP_TIMEOUT_MS` | `15000` ms | Maximum wait for a DHCP lease |
| Robot IP / subnet / gateway | — | Assigned by DHCP at runtime; no constant needed |

> **MAC address for DHCP reservation:** if you want the router to always give
> a robot the same IP, add a MAC-based reservation in your router's DHCP
> settings.  The MAC is printed in the boot log:
> ```
> I (...) WiFi: connected — IP 192.168.1.42 — remote control + telemetry enabled
> ```
> (The IP shown is the DHCP-assigned address for that boot.)

---

## 3  Firewall / router rules

| Direction        | Protocol | Port | Action |
|------------------|----------|------|--------|
| Robot → LAN      | UDP      | 9001 | Allow  |
| LAN → Robot      | UDP      | 9000 | Allow  |

No TCP, no ICMP rules required.

---

## 4  Monitor script (Python)

Save as `tools/monitor.py` and run from any machine on the same LAN:

```python
#!/usr/bin/env python3
"""Receive and display robot telemetry from UDP broadcast port 9001."""

import socket, json, datetime

LISTEN_IP   = "0.0.0.0"
LISTEN_PORT = 9001

sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
sock.bind((LISTEN_IP, LISTEN_PORT))

print(f"Listening for telemetry on UDP :{LISTEN_PORT} …")

while True:
    data, addr = sock.recvfrom(256)
    try:
        frame = json.loads(data.decode())
        ts = datetime.datetime.now().strftime("%H:%M:%S.%f")[:-3]
        print(
            f"{ts}  [{frame.get('ip', addr[0])}]  state={frame['s']:8s}  "
            f"ll={frame['ll']:4d}cm  lr={frame['lr']:4d}cm  "
            f"tl={frame['tl']:+4d}  tr={frame['tr']:+4d}  "
            f"uptime={frame['ms']/1000:.1f}s"
        )
    except (json.JSONDecodeError, KeyError) as e:
        print(f"Bad frame from {addr}: {data!r}  ({e})")
```

```bash
python3 tools/monitor.py
```

---

## 5  Send commands (Python)

The robot's IP is dynamic (DHCP-assigned).  Read it from a recent telemetry
frame or use the fleet server REST API (`POST /command/{ip}/button`).

```python
#!/usr/bin/env python3
"""Send control commands to a robot over UDP port 9000.

Obtain ROBOT_IP from a telemetry frame:
  python3 tools/monitor.py   # prints "[192.168.1.42]  state=IDLE ..."
"""

import socket, struct

ROBOT_IP   = "192.168.1.42"   # replace with the IP printed in the boot log
ROBOT_PORT = 9000

sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

def send_throttle(left: int, right: int):
    """left, right: signed int [-100, 100]"""
    pkt = struct.pack("BBBB", 0xA5, 0x01,
                      left  & 0xFF,
                      right & 0xFF)
    sock.sendto(pkt, (ROBOT_IP, ROBOT_PORT))

def send_button():
    pkt = struct.pack("BBBB", 0xA5, 0x02, 0x00, 0x00)
    sock.sendto(pkt, (ROBOT_IP, ROBOT_PORT))

# Example: press the button (start recording)
send_button()

# Example: drive forward at 50% throttle
send_throttle(50, 50)
```

> **Tip:** use the fleet server instead — `POST /command/{ip}/button` and
> `POST /command/{ip}/throttle` handle IP lookup automatically.
> See [Runbook 08 — Fleet management](08-fleet-management.md).

---

## 6  2.4 GHz band requirement

The ESP32-WROOM-32D **only supports 2.4 GHz 802.11b/g/n**.  Ensure your AP is
broadcasting on 2.4 GHz.  5 GHz-only or tri-band APs that disable 2.4 GHz
will result in the robot never connecting.

---

## 7  SSID / password encoding

`esp-wifi` expects UTF-8 SSID and password strings.  Non-ASCII characters
in the SSID (emoji, CJK, etc.) are supported as long as the router advertises
the same UTF-8 encoding.  Avoid SSIDs with embedded NUL bytes.

---

## 8  Connection timeout behaviour

If WiFi fails to connect within approximately 30 seconds (the default `esp-wifi`
scan + associate timeout), `WifiAdapter` logs an error and sets
`self.inner = None`.  The robot continues to operate in fully local mode:

- Joystick and LIDAR still work normally.
- Telemetry frames are silently discarded.
- Remote-control commands are never received.

The operator can observe this in the boot log:

```
E (...) WiFi connect failed: Disconnected
I (...) WiFi unavailable — running in local mode
I (...) State: IDLE
```

To re-attempt WiFi, power-cycle the robot.
