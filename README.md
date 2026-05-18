# Path-Following Robot — ESP32-WROOM-32D

> Bare-metal Rust firmware (`no_std`) for a LIDAR-guided path-following farm robot.  
> The robot **records** a joystick-driven path, then **replays** it autonomously
> while two TF-Luna LIDARs detect and avoid obstacles in real time.  
> WiFi telemetry and remote control are included.

---

## Table of contents

- [Requirements](#requirements)
- [Quick start](#quick-start)
- [How to flash](#how-to-flash)
- [Hardware](#hardware)
  - [Bill of materials](#bill-of-materials)
  - [Pin assignment table](#pin-assignment-table)
  - [Wiring diagram](#wiring-diagram)
- [Software architecture](#software-architecture)
  - [Layer diagram](#layer-diagram)
  - [Dependency graph](#dependency-graph)
  - [Boot sequence](#boot-sequence)
- [State machine](#state-machine)
- [WiFi protocol](#wifi-protocol)
- [Configuration reference](#configuration-reference)
- [Project structure](#project-structure)
- [Testing](#testing)
- [Documentation index](#documentation-index)

---

## Requirements

### Hardware

| Item | Spec | Notes |
|---|---|---|
| Microcontroller | ESP32-WROOM-32D | The `-32D` variant has 4 MB flash |
| USB–UART adapter | CP2102 or CH340 | Usually built into dev boards |
| Motor driver | DRV8833 | Two H-bridges; 1.5 A per channel |
| DC motors | 3–10 V, ≤1.5 A each | Two differential-drive wheels |
| LIDAR sensors | TF-Luna × 2 | 3.3 V logic; 5 V power supply |
| Joystick module | Analog XY + push button | KY-023 or equivalent |
| Power supply (logic) | 3.3 V / 500 mA | Supplied by dev board regulator |
| Power supply (motors) | 5–10 V / 3 A | Separate rail; do not share with logic |

### Software (development machine)

| Tool | Minimum version | Install |
|---|---|---|
| Rust stable | 1.88 | `rustup toolchain install stable` |
| Rust esp toolchain | latest | `cargo install espup && espup install` |
| espflash | 3.0 | `cargo install espflash` |
| Python | 3.8 | *(optional — monitor scripts only)* |

Tested on macOS 14 (arm64) and Ubuntu 22.04 (x86_64).

---

## Quick start

```bash
# ── 1. One-time toolchain setup ───────────────────────────────────────────────
cargo install espup && espup install
source ~/export-esp.sh                # add to ~/.zshrc or ~/.bashrc

cargo install espflash

# ── 2. Clone and configure ────────────────────────────────────────────────────
git clone <repo-url> path-following-robot-esp32-wroom-32d
cd path-following-robot-esp32-wroom-32d

# Edit src/config.rs — set WIFI_SSID and WIFI_PASSWORD
#   The robot obtains its IP via DHCP — no static address needed

# ── 3. Verify with host unit tests (no ESP32 needed) ──────────────────────────
cargo +stable test --lib --target aarch64-apple-darwin
# Expected: test result: ok. 19 passed; 0 failed

# ── 4. Build release firmware ─────────────────────────────────────────────────
cargo +esp build --release

# ── 5. Flash and open serial monitor ─────────────────────────────────────────
espflash flash --monitor \
  target/xtensa-esp32-none-elf/release/path-following-robot
```

---

## How to flash

### Step 1 — Enter download mode

Most ESP32 dev boards auto-reset into download mode when `espflash` opens the
port.  If your board does not:

1. Hold the **BOOT** button.
2. Press and release **RESET** / **EN**.
3. Release **BOOT**.

The blue LED stays on when download mode is active.

### Step 2 — Flash

```bash
# Auto-detect port (works on most systems)
espflash flash --monitor \
  target/xtensa-esp32-none-elf/release/path-following-robot

# Explicit port
espflash flash --port /dev/cu.usbserial-0001 --monitor \
  target/xtensa-esp32-none-elf/release/path-following-robot

# Flash without opening monitor (silent)
espflash flash \
  target/xtensa-esp32-none-elf/release/path-following-robot
```

`espflash` strips debug symbols and compresses the binary before writing.
A release build flashes in approximately 10 seconds.

### Step 3 — Verify boot

After flashing, the serial monitor shows:

```
ESP-ROM:esp32-eco3
rst:0x1 (POWERON_RESET),boot:0x13 (SPI_FAST_FLASH_BOOT)
…
I (321)  path_following_robot: === path-following-robot booting ===
I (330)  path_following_robot: LEDC: 4 × 8-bit channels @ 1000 Hz
I (335)  path_following_robot: UART: LIDAR L=UART1/GPIO9  R=UART2/GPIO16
I (340)  path_following_robot: ADC: joystick X=GPIO36  Y=GPIO39
I (342)  path_following_robot: Button: GPIO27 (active-low, internal pull-up)
I (350)  path_following_robot: WiFi connecting to "YourSSID"...
I (4230) path_following_robot: WiFi: connected — IP 192.168.1.42 — remote control + telemetry enabled
I (4231) path_following_robot: Robot ready — entering main loop at ~100 Hz
I (4232) path_following_robot: State: IDLE
```

The robot is ready when `State: IDLE` appears.  Press the joystick button to
begin recording.

### Log level

```rust
// src/bin/main.rs
// debug builds already set LevelFilter::Debug automatically
log::set_max_level(log::LevelFilter::Trace);  // most verbose
log::set_max_level(log::LevelFilter::Info);   // production default
```

---

## Hardware

### Bill of materials

| Qty | Part | Description | Approx. cost |
|---|---|---|---|
| 1 | ESP32-WROOM-32D dev board | e.g. ESP32-DevKitC-V4 | $5 |
| 1 | DRV8833 breakout | Dual H-bridge motor driver | $2 |
| 2 | TF-Luna | Micro LIDAR (0.2–8 m, 100 Hz) | $15 each |
| 1 | KY-023 joystick | Analog XY + tactile button | $1 |
| 2 | DC gear motor | 6 V, ≤1 A each | $4 each |
| 1 | 5–9 V power bank or LiPo | Motor supply | — |
| — | Jumper wires, breadboard | — | — |

### Pin assignment table

| Signal | ESP32 GPIO | Interface | Notes |
|---|---|---|---|
| Motor AIN1 | 25 | LEDC PWM ch0 | Left wheel forward |
| Motor AIN2 | 26 | LEDC PWM ch1 | Left wheel reverse |
| Motor BIN1 | 32 | LEDC PWM ch2 | Right wheel forward |
| Motor BIN2 | 33 | LEDC PWM ch3 | Right wheel reverse |
| LIDAR-L RX | 9 | UART1 RX | ⚠ In WROOM flash range — remap to 22 if issues |
| LIDAR-L TX | 10 | UART1 TX | ⚠ In WROOM flash range — remap to 23 if issues |
| LIDAR-R RX | 16 | UART2 RX | |
| LIDAR-R TX | 17 | UART2 TX | |
| Joystick X | 36 (VP) | ADC1 ch0 | Input-only, no pull resistor needed |
| Joystick Y | 39 (VN) | ADC1 ch3 | Input-only, no pull resistor needed |
| Joystick BTN | 27 | GPIO input | Active-low, internal pull-up enabled |

All GPIO signals are 3.3 V.  Do **not** connect 5 V signals directly to GPIO pins.

### Wiring diagram

```
3.3 V rail ──────────────────────────────────────────────────────────────────┐
GND rail   ──────────────────────────────────────────────────────────────────┤
                                                                              │
╔══════════════════════════╗                                                  │
║   ESP32-WROOM-32D        ║                                                  │
║                          ║   ┌──────────────────────────────────────────┐  │
║  GPIO25 ─── AIN1 ────────╫──►│                                          │  │
║  GPIO26 ─── AIN2 ────────╫──►│   DRV8833                                │  │
║  GPIO32 ─── BIN1 ────────╫──►│   (Motor Driver)                         │  │
║  GPIO33 ─── BIN2 ────────╫──►│                                          │  │
║                          ║   │  AOUT1/AOUT2 ──► Left  Motor  (DC)       │  │
║  3.3 V ──────────────────╫──►│  VCC   = 3.3 V                           │◄─┘
║  GND   ──────────────────╫──►│  VM    = motor power (5–10 V, separate!) │
║                          ║   │  BOUT1/BOUT2 ──► Right Motor  (DC)       │
║                          ║   └──────────────────────────────────────────┘
║                          ║
║  GPIO9  ◄── TX ──────────╫────  TF-Luna LIDAR (left)
║  GPIO10 ──► RX ──────────╫────  (TX from robot, usually not needed)
║  3.3 V  ──────────────────╫────  VCC  (TF-Luna needs 5 V on power pin!)
║  GND    ──────────────────╫────  GND
║                          ║      ⚠ Use level-shifter or 5 V VIN + 3.3 V data
║  GPIO16 ◄── TX ──────────╫────  TF-Luna LIDAR (right)
║  GPIO17 ──► RX ──────────╫────
║                          ║
║  GPIO36 (VP) ◄── Xout ───╫────  KY-023 Joystick  (X axis, 0–3.3 V)
║  GPIO39 (VN) ◄── Yout ───╫────                    (Y axis, 0–3.3 V)
║  GPIO27      ◄── SW  ────╫────                    (button, active-low)
║  3.3 V  ─────── VCC  ────╫────                    (VCC)
║  GND    ─────── GND  ────╫────                    (GND)
║                          ║
║  USB ◄──────────────────────── USB-UART (CP2102/CH340)  flash / serial log
╚══════════════════════════╝

Motor power (5–10 V) ──► DRV8833 VM pin  (separate from logic 3.3 V rail!)
```

> **Power note:** The DRV8833 motor supply (`VM`) must come from a separate
> 5–10 V rail (battery, power bank).  Never power motors from the ESP32 3.3 V
> regulator — the current draw will brown out the MCU and corrupt flash writes.

> **TF-Luna power note:** The TF-Luna requires **5 V** on its `VIN` power pin,
> but its UART **data lines are 3.3 V** compatible.  Use the 5 V pin on your
> dev board (or a separate 5 V rail) for TF-Luna VIN.

---

## Software architecture

### Layer diagram

```
  ┌────────────────────────────────────────────────────────────────────────────┐
  │                        src/bin/main.rs                                     │
  │              Composition root — ESP32 entry point                          │
  │   Initialises peripherals, constructs adapters, owns the Robot aggregate,  │
  │   runs the 100 Hz cooperative loop:  robot.tick(now_ms)                    │
  └──────────────────────────────┬─────────────────────────────────────────────┘
                                 │ constructs & owns
  ┌──────────────────────────────▼─────────────────────────────────────────────┐
  │                       DOMAIN  (no_std, no esp-hal)                         │
  │  ┌────────────────────────────────────────────────────────────────────┐    │
  │  │  Robot<M: MotorPort, L: DistancePort, I: InputPort, W = NoWifi>    │    │
  │  │                                                                    │    │
  │  │  FSM:  IDLE → RECORD → READY → PLAY ⇄ AVOIDING → HALT            │    │
  │  │  Path: PathBuffer  (heapless::Vec<PathCommand, 512>)               │    │
  │  └────────────────────────────────────────────────────────────────────┘    │
  └──────────┬──────────────────────────────────────────────────┬──────────────┘
             │ calls via trait                                   │ calls via trait
  ┌──────────▼────────────────────────┐   ┌─────────────────────▼──────────────┐
  │            PORTS                  │   │             PORTS                   │
  │  MotorPort        — set_throttle  │   │  RemoteControlPort — poll_button    │
  │  DistancePort     — read_cm       │   │                    — poll_throttle  │
  │  InputPort        — poll_button   │   │  TelemetryPort     — send(&frame)   │
  │                   — read_throttle │   └──────────────┬──────────────────────┘
  └──────────┬────────────────────────┘                  │
             │ implemented by (xtensa only)               │ implemented by (xtensa only)
  ┌──────────▼────────────────────────┐   ┌──────────────▼──────────────────────┐
  │       ADAPTERS / esp32            │   │       ADAPTERS / esp32               │
  │                                   │   │                                      │
  │  Drv8833         LEDC PWM × 4     │   │  WifiAdapter                        │
  │  TfLuna (×2)     UART1 / UART2    │   │    esp-wifi (ISR-driven)             │
  │  Esp32Joystick   ADC1 + GPIO      │   │    smoltcp 0.12 (UDP sockets)        │
  │                                   │   │    block_on spin-loop (connect)      │
  └───────────────────────────────────┘   └──────────────────────────────────────┘
             │                                           │
  ┌──────────▼───────────────────────────────────────────▼──────────────────────┐
  │                          esp-hal  1.1.1                                      │
  │  LEDC · UART · ADC1 · GPIO · TIMG · RNG · RADIO_CLK · WIFI peripheral       │
  └──────────────────────────────────────────────────────────────────────────────┘
             │
  ┌──────────▼───────────────────────────────────────────────────────────────────┐
  │                     ESP32-WROOM-32D Hardware                                 │
  │  DRV8833 motors · TF-Luna LIDARs · KY-023 joystick · 2.4 GHz WiFi           │
  └──────────────────────────────────────────────────────────────────────────────┘
```

The domain layer has **zero** `esp-hal` imports.  The entire `adapters/esp32/`
subtree is gated `#[cfg(target_arch = "xtensa")]` so it is never compiled on
the host.

### Dependency graph

```
path-following-robot (bin)
├── path_following_robot (lib)
│   ├── domain::robot       — FSM, no external deps beyond heapless + log
│   ├── domain::path        — PathBuffer (heapless::Vec)
│   ├── domain::state       — RobotState enum
│   ├── ports::*            — trait definitions (zero deps)
│   └── adapters::esp32::*  — [xtensa only]
│       ├── esp-hal  1.1.1
│       ├── esp-wifi ~0.14  (default-features=false, no builtin-scheduler)
│       ├── smoltcp  0.12   (proto-ipv4, socket-udp, medium-ethernet)
│       ├── esp-alloc 0.8   (global allocator for WiFi heap)
│       └── static_cell 2
├── log       0.4  (facade; backend = esp-println on device, env_logger in tests)
└── heapless  0.8  (PathBuffer, telemetry scratch buffer)
```

### Boot sequence

```
power on
    │
    ▼
esp-idf bootloader (in ROM)
    │  verifies flash partition table
    ▼
application binary loaded
    │
    ▼
esp_alloc::heap_allocator!(72 KiB)    ← heap for WiFi / smoltcp
    │
    ▼
esp_hal::init(CpuClock::max())        ← 240 MHz, peripherals unlocked
    │
    ▼
LEDC timer + 4 PWM channels           ← DRV8833 motor control ready
    │
    ▼
UART1 (GPIO9) + UART2 (GPIO16)        ← TF-Luna LIDAR streams active
    │
    ▼
ADC1 (GPIO36, GPIO39) + GPIO27        ← joystick axes + button ready
    │
    ▼
WifiAdapter::connect()                ← block_on spin-loop, ≤15 s
    │  success: DHCP-assigned IP (embedded in every telemetry frame)
    │  failure: WifiAdapter degraded to NoWifi (robot still works locally)
    ▼
Robot::new_with_wifi(motors, lidar_l, lidar_r, joystick, wifi)
    │
    ▼
loop {                                ← 100 Hz cooperative loop
    now_ms = boot.elapsed().as_millis()
    robot.tick(now_ms)               ← FSM + sensor + WiFi poll
    delay.delay_millis(10)
}
```

---

## State machine

```
                    ┌─────────────────────────────────────────────────────────┐
                    │                  button / WiFi button                   │
                    │                                                         ▼
              ┌─────┴──┐  button   ┌────────┐  button   ┌───────┐  button  ┌──────┐
  power on ──►│  IDLE  │──────────►│ RECORD │──────────►│ READY │─────────►│ PLAY │
              └────────┘           └────────┘           └───────┘          └──┬───┘
                                   records path                   ▲            │ obstacle
                                   (512 cmds max)                 │            │ < 80 cm
                                                           PLAY resumed        ▼
                                                           when clear   ┌──────────┐
                                                                        │ AVOIDING │
                   ┌──────┐ ◄── path complete ──────────────────────────└──────────┘
                   │ HALT │ ◄── buffer overflow (from RECORD)
                   └──────┘ ◄── avoidance timeout > 10 s (from AVOIDING)
                   (power cycle to exit)
```

| State | Motors | LIDARs | Joystick | WiFi button | WiFi throttle |
|---|---|---|---|---|---|
| `IDLE` | coast | ignored | ignored | → `RECORD` | ignored |
| `RECORD` | joystick | triggers `HALT` on overflow | drives + records | → `READY` | overrides drive |
| `READY` | coast | ignored | ignored | → `PLAY` or `IDLE` | ignored |
| `PLAY` | replays path | → `AVOIDING` if < 80 cm | ignored | ignored | ignored |
| `AVOIDING` | manoeuvre | clears → `PLAY` | ignored | ignored | ignored |
| `HALT` | coast | ignored | ignored | ignored | ignored |

---

## WiFi protocol

The robot broadcasts **JSON telemetry** and accepts **4-byte binary commands** over UDP.
All communication is on the LAN; no cloud dependency.

### Telemetry  ← robot  (UDP broadcast, port `9001`, 200 ms interval)

```json
{"s":"PLAY","ll":125,"lr":98,"tl":50,"tr":50,"ms":12345,"ip":"192.168.1.42"}
```

| Field | Type | Range | Meaning |
|---|---|---|---|
| `s` | string | — | FSM state: `IDLE` `RECORD` `READY` `PLAY` `AVOIDING` `HALT` |
| `ll` | int | 0–1200, or −1 | Left LIDAR distance (cm); −1 = stale / sensor absent |
| `lr` | int | 0–1200, or −1 | Right LIDAR distance (cm); −1 = stale / sensor absent |
| `tl` | int | −100 … 100 | Left motor throttle |
| `tr` | int | −100 … 100 | Right motor throttle |
| `ms` | int | 0 … 2³² | Uptime in milliseconds since boot |
| `ip` | string | — | DHCP-assigned IPv4 address of this robot |

### Commands → robot  (UDP unicast to robot's current IP, port `9000`)

```
Byte 0: 0xA5  (magic / framing byte)
Byte 1: type
Byte 2: v1
Byte 3: v2
```

| `type` | Command | `v1` | `v2` | Effect |
|---|---|---|---|---|
| `0x01` | throttle | left as i8 (cast u8) | right as i8 (cast u8) | Override drive in `RECORD` state |
| `0x02` | button | ignored | ignored | Synthetic button press — same as physical |

See [`docs/runbooks/03-wifi-setup.md`](docs/runbooks/03-wifi-setup.md) for
IP configuration and Python monitor / control scripts.

---

## Configuration reference

All tunable constants live in **`src/config.rs`**.  A full rebuild is required
after any change.

| Constant | Default | Unit | Description |
|---|---|---|---|
| `WIFI_SSID` | `"your_ssid"` | — | AP SSID (**must change before flashing**) |
| `WIFI_PASSWORD` | `"your_password"` | — | WPA2 passphrase |
| `WIFI_CMD_PORT` | `9000` | UDP port | Inbound remote-control port |
| `WIFI_TEL_PORT` | `9001` | UDP port | Outbound telemetry port |
| `TELEMETRY_INTERVAL_MS` | `200` | ms | Telemetry send interval |
| `WIFI_DHCP_TIMEOUT_MS` | `15000` | ms | Max wait for DHCP lease before offline mode |
| `WIFI_HEAP_SIZE` | `73728` | bytes | Heap reserved for WiFi + smoltcp |
| `OBSTACLE_CM` | `80` | cm | LIDAR distance that triggers AVOIDING |
| `CLEAR_CM` | `100` | cm | Both LIDARs must exceed this to resume PLAY |
| `WARN_CM` | `120` | cm | Advisory log threshold (no state change) |
| `AVOID_BACK_MS` | `200` | ms | Reverse duration at avoidance start |
| `AVOID_TURN_MS` | `300` | ms | Turn duration after reversing |
| `AVOID_TIMEOUT_MS` | `10000` | ms | Max time in AVOIDING before HALT |
| `PATH_CAPACITY` | `512` | cmds | Max recorded path commands |
| `PATH_CMD_INTERVAL_MS` | `20` | ms | Joystick sampling interval during RECORD |
| `STALE_TICKS` | `50` | ticks | LIDAR ticks without data before stale |
| `DEBOUNCE_MS` | `50` | ms | Minimum time between button events |
| `LOOP_MS` | `10` | ms | Main loop period (100 Hz) |
| `PWM_FREQ_HZ` | `1000` | Hz | DRV8833 PWM carrier frequency |

---

## Project structure

```
path-following-robot-esp32-wroom-32d/
│
├── src/
│   ├── bin/
│   │   └── main.rs               # ESP32 entry point; composition root
│   │                             #   — inits peripherals, constructs adapters,
│   │                             #     runs 100 Hz cooperative loop
│   ├── lib.rs                    # Crate root; cfg-gates adapters to xtensa only
│   ├── config.rs                 # All tunable constants (GPIOs, thresholds, WiFi)
│   │
│   ├── domain/                   # Pure Rust — zero esp-hal imports
│   │   ├── mod.rs
│   │   ├── state.rs              # RobotState (6 variants) + ObstacleSide
│   │   ├── path.rs               # PathCommand + PathBuffer (heapless, 512 entries)
│   │   └── robot.rs              # Robot<M,L,I,W=NoWifi> FSM aggregate
│   │                             #   + NoWifi zero-sized type
│   │                             #   + 19 unit tests (all run on host)
│   │
│   ├── ports/                    # Port trait definitions — the hexagonal boundary
│   │   ├── mod.rs
│   │   ├── motors.rs             # MotorPort: set_throttle(left, right)
│   │   ├── distance.rs           # DistancePort: read_cm() -> Option<u16>
│   │   ├── input.rs              # InputPort: poll_button(), read_throttle()
│   │   ├── remote_control.rs     # RemoteControlPort: poll_network, poll_button,
│   │   │                         #                    poll_throttle
│   │   └── telemetry.rs          # TelemetryPort: send(&TelemetryFrame)
│   │                             #   + TelemetryFrame (Copy struct)
│   │
│   └── adapters/
│       └── esp32/                # Concrete hardware bindings [xtensa only]
│           ├── mod.rs
│           ├── drv8833.rs        # DRV8833 H-bridge via LEDC 8-bit PWM × 4
│           │                     #   fast-decay mode, signed throttle [-100,100]
│           ├── tf_luna.rs        # TF-Luna LIDAR via UART (9-byte frames, 100 Hz)
│           │                     #   incremental parser, checksum verify, staleness
│           ├── joystick.rs       # KY-023 joystick via ADC1 + GPIO button
│           │                     #   dead-zone filter, debounce, centred throttle
│           └── wifi.rs           # WifiAdapter: esp-wifi + smoltcp UDP
│                                 #   block_on connect, poll_network, send_telemetry
│
├── docs/
│   ├── adr/
│   │   ├── 001-hexagonal-architecture.md
│   │   └── 002-no-embassy-bare-metal-wifi.md
│   └── runbooks/
│       ├── 01-prerequisites.md
│       ├── 02-build-and-flash.md
│       ├── 03-wifi-setup.md
│       ├── 04-monitoring.md
│       ├── 05-troubleshooting.md
│       ├── 06-hardware-wiring.md
│       └── 07-development-guide.md
│
├── Cargo.toml                    # Dependencies; esp-hal pinned to =1.1.1
├── Cargo.lock
├── build.rs                      # esp-hal build script hook
├── rust-toolchain.toml           # channel = "esp"
├── .cargo/config.toml            # target = xtensa-esp32-none-elf
├── Dockerfile                    # Multi-stage build for telemetry-server
├── docker-compose.yml            # Postgres 16 + telemetry-server
└── .dockerignore
```

---

## Testing

### Host unit tests (no hardware)

```bash
cargo +stable test --lib --target aarch64-apple-darwin
```

19 tests covering all FSM state transitions and edge cases.  These run on any
macOS/Linux machine in ~5 seconds.  No ESP32 required.

```
test domain::robot::tests::avoiding_back_phase_drives_backward ... ok
test domain::robot::tests::avoiding_both_sides_turns_right_by_convention ... ok
test domain::robot::tests::avoiding_left_obstacle_turns_right ... ok
test domain::robot::tests::avoiding_resumes_play_when_clear ... ok
test domain::robot::tests::avoiding_resume_drives_current_command ... ok
test domain::robot::tests::avoiding_right_obstacle_turns_left ... ok
test domain::robot::tests::avoiding_timeout_triggers_halt ... ok
test domain::robot::tests::halt_coasts_motors ... ok
test domain::robot::tests::halt_repeats_coast_on_subsequent_ticks ... ok
test domain::robot::tests::idle_button_transitions_to_record ... ok
test domain::robot::tests::play_advances_commands_by_duration ... ok
test domain::robot::tests::play_drives_first_command_immediately ... ok
test domain::robot::tests::play_halts_when_path_complete ... ok
test domain::robot::tests::play_obstacle_triggers_avoiding ... ok
test domain::robot::tests::play_stale_sensor_does_not_trigger_avoiding ... ok
test domain::robot::tests::ready_button_with_path_transitions_to_play ... ok
test domain::robot::tests::ready_with_empty_path_returns_to_idle ... ok
test domain::robot::tests::record_button_transitions_to_ready ... ok
test domain::robot::tests::record_buffer_overflow_triggers_halt ... ok

test result: ok. 19 passed; 0 failed
```

### Cargo check (xtensa cross-compile)

```bash
cargo +esp check
```

Verifies the xtensa-only adapter code (WiFi, LEDC, UART, ADC) compiles
without a flash/run cycle.

---

## Documentation index

| Document | Purpose |
|---|---|
| [ADR-001 — Hexagonal architecture](docs/adr/001-hexagonal-architecture.md) | Why the domain is HAL-free; generics vs dyn; NoWifi default |
| [ADR-002 — No Embassy / bare-metal WiFi](docs/adr/002-no-embassy-bare-metal-wifi.md) | Why Embassy was removed; block_on mechanics; Cargo fix |
| [ADR-003 — DHCP + fixed-endpoint server](docs/adr/003-dhcp-dynamic-robot-ip-fixed-server.md) | Why robots use DHCP; IP embedded in telemetry; fleet server design |
| [Runbook 01 — Prerequisites](docs/runbooks/01-prerequisites.md) | Rust stable + esp toolchain; espflash; USB-UART drivers |
| [Runbook 02 — Build & Flash](docs/runbooks/02-build-and-flash.md) | config.rs WiFi/GPIO; build commands; binary size; log levels |
| [Runbook 03 — WiFi setup](docs/runbooks/03-wifi-setup.md) | Network topology; DHCP; firewall; Python monitor + control scripts |
| [Runbook 04 — Monitoring](docs/runbooks/04-monitoring.md) | Log prefix legend; telemetry fields; state transitions; LIDAR health |
| [Runbook 05 — Troubleshooting](docs/runbooks/05-troubleshooting.md) | 15+ failure scenarios: build, flash, WiFi, LIDAR, motors, FSM |
| [Runbook 06 — Hardware wiring](docs/runbooks/06-hardware-wiring.md) | Detailed wiring guide, power rails, level-shifting |
| [Runbook 07 — Development guide](docs/runbooks/07-development-guide.md) | Adding sensors, porting to new MCU, extending the FSM |
| [Runbook 08 — Fleet management server](docs/runbooks/08-fleet-management.md) | telemetry-server binary; Docker; UI; SSE; Postgres log storage |
