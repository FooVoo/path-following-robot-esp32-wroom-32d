# Runbook 02 — Build and Flash

> **Audience:** Developers building and deploying firmware.
>
> **See also:** [Runbook 10 — Step-by-Step Flashing and Wiring Guide](10-flashing-and-wiring-guide.md)
> for a full walkthrough including hardware wiring, download mode, port detection, and first-boot verification.

---

## 1  Configure the build

All tunable constants are in **`src/config.rs`**.  Edit this file before
building.

### 1.1  WiFi credentials (required)

```rust
pub const WIFI_SSID:     &str = "MyNetwork";
pub const WIFI_PASSWORD: &str = "secret";
```

### 1.2  Network addressing

The robot obtains its IP address via **DHCP** automatically — no static address
constants are needed.  The only optional tuning:

```rust
/// Max ms to wait for a DHCP lease before falling back to offline mode.
pub const WIFI_DHCP_TIMEOUT_MS: u64 = 15_000;
```

The DHCP-assigned IP is printed in the boot log and embedded in every telemetry
frame (`"ip"` field).  If you want a consistent IP across reboots, add a
MAC-based DHCP reservation in your router settings.

### 1.3  GPIO mapping (if hardware differs)

```rust
pub const MOTOR_A_IN1_GPIO: u8 = 25;
pub const MOTOR_A_IN2_GPIO: u8 = 26;
pub const MOTOR_B_IN1_GPIO: u8 = 32;
pub const MOTOR_B_IN2_GPIO: u8 = 33;
pub const LIDAR_L_RX_GPIO:  u8 = 9;   // remap if using WROOM-32D quad-SPI conflict
pub const LIDAR_R_RX_GPIO:  u8 = 16;
pub const JOYSTICK_BTN_GPIO: u8 = 27;
```

---

## 2  Run host unit tests

Always run tests before flashing to catch logic regressions without needing
hardware:

```bash
cargo +stable test --lib --target aarch64-apple-darwin
# Expected: test result: ok. 47 passed; 0 failed
```

---

## 3  Build the firmware

```bash
# Development build (faster compile, larger binary — still fits in 4 MB flash)
cargo +esp build

# Release build (recommended for deployment — LTO, size-optimised)
cargo +esp build --release
```

The ELF binary lands at:

```
target/xtensa-esp32-none-elf/release/path-following-robot
```

### Check binary size

```bash
cargo +esp size --release -- -A
```

Typical figures:

| Section  | Size    | Notes                          |
|----------|---------|--------------------------------|
| `.text`  | ~190 KB | Code + esp-wifi ISR handlers   |
| `.rodata`| ~15 KB  | String constants               |
| `.data`  | ~4 KB   | Mutable statics                |
| Heap     | 72 KB   | WiFi heap (runtime, not flash) |

---

## 4  Flash the firmware

Connect the ESP32 via USB.  Put it into **download mode** if the board does
not have auto-reset:
- Hold BOOT, press RESET, release RESET, release BOOT.

```bash
# Flash + open serial monitor immediately
cargo +esp run --release

# Or: flash only (specify port explicitly if auto-detect fails)
espflash flash --port /dev/cu.usbserial-0001 \
               target/xtensa-esp32-none-elf/release/path-following-robot

# Or: flash + monitor (espflash built-in)
espflash flash --monitor target/xtensa-esp32-none-elf/release/path-following-robot
```

---

## 5  Serial monitor (UART0, 115 200 baud)

The firmware logs to UART0 via `esp-println`.  Log levels:

| Level   | Content |
|---------|---------|
| `INFO`  | State transitions, WiFi connect/disconnect events |
| `DEBUG` | LIDAR frame receipt, command receipt, tick timing |
| `TRACE` | Raw UART bytes, ADC readings, every tick entry     |

To filter noise during development:

```bash
# Show only INFO and above (espflash monitor)
espflash monitor --port /dev/cu.usbserial-0001

# Or with minicom / screen
screen /dev/cu.usbserial-0001 115200
```

Set the log level at compile time in `src/bin/main.rs`:

```rust
// Show INFO and above (default)
log::set_max_level(log::LevelFilter::Info);

// Enable DEBUG for sensor troubleshooting
log::set_max_level(log::LevelFilter::Debug);

// Enable TRACE for byte-level protocol debugging
log::set_max_level(log::LevelFilter::Trace);
```

---

## 6  Typical boot log

```
I (321) path_following_robot: WiFi connecting to "MyNetwork"...
I (4231) path_following_robot: WiFi: connected — IP 192.168.1.42 — remote control + telemetry enabled
I (4232) path_following_robot: State: IDLE
D (4234) path_following_robot: tick 0 — IDLE, lidar_l=None, lidar_r=None
```

The robot is in `IDLE` and ready for operator input when the second `State:`
line appears.

---

## 7  Over-the-air update (OTA)

OTA is not implemented in the current firmware version.  To update:

1. Run `cargo +esp build --release`
2. Flash via USB as described in step 4
3. Reboot the robot

This is a deliberate simplification — the robot is expected to be accessible
for flashing in a farm maintenance scenario.
