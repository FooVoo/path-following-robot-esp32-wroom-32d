# 09 — Wokwi Simulation

Simulate the path-following robot firmware in [Wokwi](https://wokwi.com) without
real hardware.  The simulation binary (`path-following-robot-sim`) replaces the
I2C LIDAR sensors with a stub oscillator and disables WiFi, making the full FSM
runnable in the browser or VS Code.

---

## Prerequisites

| Requirement | Version | Install |
|---|---|---|
| Rust + esp toolchain | (from `rust-toolchain.toml`) | `rustup toolchain install esp` |
| `cargo-espflash` | any | `cargo install cargo-espflash` |
| Wokwi VS Code extension | ≥ 2.x | VS Code Extensions marketplace |
| Wokwi licence | free tier or pro | <https://wokwi.com/pricing> |

---

## 1  Build the simulation binary

```sh
# Via the project alias (recommended):
cargo build-sim

# Explicit form:
cargo +esp build --features sim --bin path-following-robot-sim
```

The ELF is written to:

```
target/xtensa-esp32-none-elf/debug/path-following-robot-sim
```

This path is already configured in `wokwi.toml`.

---

## 2  Run in VS Code (Wokwi extension)

1. Open the project folder in VS Code.
2. Install the **Wokwi for VS Code** extension if not already present.
3. Press **F1** → **Wokwi: Start Simulator** (or click the Wokwi icon in the
   status bar).
4. The extension reads `wokwi.toml` and `diagram.json` automatically.

The simulation starts immediately.  The ESP32 serial output is shown in the
**Wokwi** terminal pane.

---

## 3  Run with `wokwi-cli` (CI / headless)

```sh
# Install CLI (npm):
npm install -g @wokwi/cli

# Export token from https://wokwi.com/dashboard/ci
export WOKWI_CLI_TOKEN=<your-token>

# Validate the diagram:
wokwi-cli lint

# Run for 30 seconds then exit 0:
wokwi-cli simulate --timeout 30000
```

---

## 4  Simulated circuit

`diagram.json` contains three components:

| Component | Wokwi type | Purpose |
|---|---|---|
| ESP32 DevKit V1 | `wokwi-esp32-devkit-v1` | MCU |
| 1602 LCD | `wokwi-lcd1602` | FSM state + LIDAR/throttle display |
| Analog joystick | `wokwi-analog-joystick` | FSM button + drive throttle |

**LCD pin mapping**

| HD44780 pin | ESP32 GPIO | Wokwi conn |
|---|---|---|
| RS | 5 | `esp:D5` → `lcd1:RS` |
| EN | 4 | `esp:D4` → `lcd1:E` |
| D4 | 13 | `esp:D13` → `lcd1:D4` |
| D5 | 14 | `esp:D14` → `lcd1:D5` |
| D6 | 15 | `esp:D15` → `lcd1:D6` |
| D7 | 2 | `esp:D2` → `lcd1:D7` |

**Joystick pin mapping**

| Joystick pin | ESP32 GPIO | Notes |
|---|---|---|
| HORZ (X) | 36 (VP) | Left / right drive |
| VERT (Y) | 39 (VN) | Forward / reverse drive |
| SEL (button) | 27 | Click to advance FSM state |

---

## 5  Operating the simulation

The robot FSM requires three button presses to reach `PLAY` state:

```
IDLE ──(btn)──► RECORD ──(btn)──► READY ──(btn)──► PLAY
```

1. **IDLE** — LCD row 0 shows `IDLE`.
   Click the joystick centre once → enters `RECORD`.
2. **RECORD** — Move the joystick to steer.  Throttle commands are buffered.
   Click once → enters `READY`.
3. **READY** — Path recorded.
   Click once → enters `PLAY`.
4. **PLAY** — Robot replays the recorded path.
   The stub LIDAR oscillates: 4 s at 200 cm (safe) → 1 s at 50 cm (obstacle).
   On each obstacle phase the robot enters `AVOIDING`, then returns to `PLAY`.

**DIRECT mode** — from `IDLE`, hold the button for ≥ 1 s then release to enter `DIRECT`.
Joystick movements are passed straight to the motors with no path recording or LIDAR
checks.  LCD row 1 switches to live throttle feedback: `L +75 R -50` (left / right,
signed, always 16 chars wide) so the operator can verify the joystick is reaching the
motors.  Press the button once to exit back to `IDLE`.

---

## 6  Known limitations

| Limitation | Reason | Workaround |
|---|---|---|
| LIDAR data is simulated | TCA9548A mux not supported by Wokwi; VL53L0X omitted | `StubDistance` oscillates 200 cm → 50 cm (5 s cycle) |
| WiFi / telemetry disabled | `NoWifi` no-ops; no RTOS started | Use serial log output in VS Code / wokwi-cli |
| Motor driver not visualised | DRV8833 PWM is driven but no motor component in diagram | Add `wokwi-dc-motor` to `diagram.json` if visual feedback is needed |
| Stepper not FSM-driven | `_stepper` is initialised but `Uln2003::step()` is never called in the FSM | Call `stepper.step()` in the main loop for custom sequences |
| No heap allocator for WiFi | WiFi stack removed; only 8 KiB heap allocated | Sufficient for prost types; increase if other alloc users are added |

---

## 7  Switching to a release build

```sh
cargo +esp build --features sim --release --bin path-following-robot-sim
```

Update `wokwi.toml` to point at the release ELF:

```toml
firmware = "target/xtensa-esp32-none-elf/release/path-following-robot-sim"
elf      = "target/xtensa-esp32-none-elf/release/path-following-robot-sim"
```

---

## 8  Extending the diagram

To add visual motor feedback, append four `wokwi-led` components to `diagram.json`
and connect them to GPIO 25/26 (left motor) and 32/33 (right motor).  See the
Wokwi component reference at <https://docs.wokwi.com/parts/wokwi-led>.
