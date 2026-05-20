# Runbook 05 — Troubleshooting

---

## Build issues

### `Could not find openssl via pkg-config` when building `telemetry-server`

```
Could not find openssl via pkg-config: pkg-config has not been configured to
support cross-compilation.
$HOST = aarch64-apple-darwin
$TARGET = xtensa-esp32-none-elf
openssl-sys = 0.9.x
```

**Cause:** `.cargo/config.toml` sets `[build] target = "xtensa-esp32-none-elf"` so
that firmware builds work out of the box.  Running `cargo build --features
host-server` without an explicit `--target` override inherits the ESP32 target,
and `openssl-sys` (previously pulled in by sqlx's `native-tls` feature) cannot
cross-compile to a bare-metal target.

**Fix (already applied):** `sqlx` is now configured with `runtime-tokio-rustls`
(pure-Rust TLS) instead of `runtime-tokio-native-tls`, which eliminates the
OpenSSL dependency entirely.

**Correct build commands:**

```bash
# macOS arm64
cargo +stable build-server
# equivalent to:
cargo +stable build --features host-server --bin telemetry-server \
      --target aarch64-apple-darwin

# Linux x86_64
cargo +stable build-server-linux
# equivalent to:
cargo +stable build --features host-server --bin telemetry-server \
      --target x86_64-unknown-linux-gnu
```

**Never** run `cargo build --features host-server` without `--target` in this
project — the `.cargo/config.toml` default target is `xtensa-esp32-none-elf`.

---

### `error: feature __esp_wifi_builtin_scheduler not found in package esp-hal`

**Cause:** `esp-wifi` default features include `builtin-scheduler`, which
requires an unreleased esp-hal feature.

**Fix:** Ensure `Cargo.toml` has `default-features = false` on `esp-wifi`:

```toml
esp-wifi = { version = "~0.14", default-features = false,
             features = ["esp32", "wifi", "smoltcp", "esp-alloc"] }
```

---

### `cargo +stable test` fails to compile `esp-hal`

**Cause:** `esp-hal`'s `build.rs` panics when the target is not Xtensa.

**Fix:** Always test with the explicit host target:

```bash
cargo +stable test --lib --target aarch64-apple-darwin   # macOS/ARM
cargo +stable test --lib --target x86_64-unknown-linux-gnu  # Linux x86
```

Never run `cargo test` without `--lib` and `--target` in this project.

---

### `esp` toolchain not found

```
error: toolchain 'esp' is not installed
```

**Fix:**

```bash
cargo install espup
espup install
source ~/export-esp.sh
```

---

### `failed to find tool "xtensa-esp-elf-gcc"` when building firmware

```
CFLAGS_xtensa-esp32-none-elf = None
cargo:warning=Compiler family detection failed due to error: ToolNotFound:
  failed to find tool "xtensa-esp-elf-gcc": No such file or directory
```

**Cause:** The Xtensa GCC cross-compiler is installed under the `esp` rustup toolchain
directory but is not on `PATH`.  This is required for any ESP32 firmware build.

**Fix:** Source the environment script before any esp build:

```bash
source ~/export-esp.sh          # sets PATH and LIBCLANG_PATH
cargo +esp build-firmware
```

Add it to your shell profile (`~/.zshrc` / `~/.bashrc`) to avoid having to do
this manually each session:

```bash
echo 'source ~/export-esp.sh' >> ~/.zshrc
```

---

### `error: no such command: +stable` (or `+esp`) when using a cargo alias

```
error: no such command: `+stable`
help: invoke `cargo` through `rustup` to handle `+toolchain` directives
```

**Cause:** Cargo expands aliases before the rustup shim can interpret a
`+toolchain` prefix.  An alias whose *value* begins with `+stable` or `+esp`
will never work, regardless of how it is invoked.

**Fix (already applied):** All aliases in `.cargo/config.toml` no longer embed
`+toolchain` in their values.  The toolchain must come from the *caller*:

```bash
cargo +stable test-host          # ✅ rustup switches to stable, then expands alias
cargo +stable build-server       # ✅
cargo +esp    build-firmware     # ✅  (also requires source ~/export-esp.sh)
```

---

### `undefined reference to 'esp_rtos_yield_task'` (and other `esp_rtos_*` symbols)

```
ld: libesp_radio.rlib: undefined reference to 'esp_rtos_yield_task'
ld: libesp_radio.rlib: undefined reference to 'esp_rtos_semaphore_create'
... (many more esp_rtos_* symbols)
```

**Cause:** `esp-rtos` only generates the RTOS compatibility symbols for
`esp-radio` when its `esp-radio` Cargo feature is explicitly enabled.  Without
it, `esp_rtos_*` are never emitted, causing link failures.

**Fix (already applied):** `Cargo.toml` now enables `esp-radio` on `esp-rtos`:

```toml
esp-rtos = { version = "0.3", features = ["esp32", "esp-radio"] }
```



**Cause:** Building with the wrong toolchain (`stable` instead of `esp`).

**Fix:** The `rust-toolchain.toml` in the project root sets `channel = "esp"`.
Ensure you are inside the project directory when running `cargo build`:

```bash
cd path-following-robot-esp32-wroom-32d
cargo build --release   # automatically picks up rust-toolchain.toml
```

---

### Binary too large for flash

```
Error: the active partition is too small for this binary
```

**Fix:** Always use `--release`:

```bash
cargo +esp build --release
```

Dev builds are not size-optimised and may exceed the 1 MB application partition.

---

## Flashing issues

### `espflash` cannot find the device

```
Error: Failed to open serial port: /dev/ttyUSB0
```

**Fix (Linux):** Add user to `dialout` group and re-log:

```bash
sudo usermod -aG dialout $USER
# log out and back in
```

**Fix (macOS):** Install the CP2102 / CH340 driver (see
[`docs/runbooks/01-prerequisites.md`](01-prerequisites.md)).

---

### Stuck in download mode (no boot after flash)

**Symptom:** Board keeps waiting for flash data, never boots.

**Fix:** Manually reset after flashing:

```bash
espflash flash target/xtensa-esp32-none-elf/release/path-following-robot
# then press the RESET button on the board
```

Or use `espflash flash --monitor` which auto-resets after flashing.

---

## WiFi issues

### Robot never connects (boot log shows timeout)

```
E (...) WiFi connect failed: Disconnected
I (...) WiFi unavailable — running in local mode
```

Checklist:

1. **SSID / password** in `src/config.rs` — check for typos and trailing spaces.
2. **2.4 GHz band** — the ESP32-WROOM-32D only supports 2.4 GHz.  Confirm your
   AP broadcasts on 2.4 GHz (not 5 GHz only).
3. **Signal strength** — bring the robot closer to the AP during initial setup.
4. **AP isolation** — some APs block peer-to-peer traffic (AP isolation / guest
   network).  Disable it for the robot's SSID.
5. **DHCP failure** — if the router does not respond within `WIFI_DHCP_TIMEOUT_MS`
   (15 s default), the robot falls back to offline mode.  Confirm the DHCP server
   is enabled on your AP and that the robot is within range.

---

### Telemetry not arriving at the monitor PC

1. Confirm the robot is connected: `State: IDLE` appears in the serial log.
2. Confirm both devices are on the **same subnet** (`192.168.1.x/24`).
3. Check the firewall on the monitor machine allows inbound UDP 9001.
4. Try `nc -u -l 9001` to manually listen:
   ```bash
   nc -u -l 9001
   ```
   A JSON line should appear every 200 ms.
5. If still nothing: check the robot's assigned IP in the serial log (printed
   after DHCP succeeds) and ping it from the monitor machine:
   ```bash
   ping <robot-ip-from-serial-log>
   ```
   If ping fails, the robot is either not connected or on a different subnet.

---

### Commands not being received by the robot

1. Confirm the robot's current IP from the serial log (printed on boot after
   DHCP succeeds, also shown in every telemetry frame's `"ip"` field).
2. Send a test packet with `nc` to the robot's IP:
   ```bash
   printf '\xa5\x02\x00\x00' | nc -u -w1 <robot-ip> 9000
   ```
3. The serial log should show:
   ```
   D (...) command received: button
   ```
4. If no log line, check that the robot is in a state where button presses are
   meaningful (not `HALT`).

---

## LIDAR issues

### LIDAR readings are always stale (`ll=-1` in telemetry)

```
W (...) lidar_l: stale
```

1. **Wiring** — verify TF-Luna TX → ESP32 RX GPIO (pin 9 for LIDAR-L, 16 for
   LIDAR-R).  Note: the TF-Luna TX goes to ESP32 RX; the labels are from the
   sensor's perspective.
2. **GPIO conflict** — GPIO 9/10 overlap with the WROOM-32D's quad-SPI flash.
   Remap LIDAR-L to GPIO 22:
   ```rust
   pub const LIDAR_L_RX_GPIO: u8 = 22;
   ```
3. **Baud rate** — TF-Luna ships at 115 200.  If it was reconfigured, update
   `LIDAR_BAUD_RATE` in `config.rs`.
4. **Power** — TF-Luna requires 5 V.  Check the supply rail.  The data pins
   are 3.3 V tolerant.
5. **Enable DEBUG log** and look for `lidar_l: bad checksum` messages.  Many
   consecutive checksum errors indicate noise on the UART line (add a 33 Ω
   series resistor on the TX line of the TF-Luna).

---

### LIDAR triggers avoidance on open ground (false positives)

Increase the obstacle threshold in `src/config.rs`:

```rust
pub const OBSTACLE_DISTANCE_CM: u16 = 50;   // increase to 70 or 80
```

Rebuild and reflash.

---

## Motor issues

### Motors not moving

1. Check DRV8833 power supply (VM pin) — motor voltage should be 3.3–10 V.
2. Verify PWM GPIO assignments in `config.rs` match your wiring.
3. Enable `DEBUG` log and look for:
   ```
   D (...) drv8833: set L=50 R=50
   ```
   If the line appears but the motors don't move, the fault is electrical.
4. Check the DRV8833 `FAULT` pin — it should be HIGH.  If LOW, the driver is
   in thermal or overcurrent shutdown.  Disconnect and let it cool.

### One motor runs at full speed regardless of throttle

The LEDC channel initialisation failed silently.  Enable `ERROR` level logging:

```rust
log::set_max_level(log::LevelFilter::Error);
```

Look for:
```
E (...) drv8833: channel config error
```

This can occur if the GPIO pin is already in use by another peripheral.

---

## State machine issues

### Robot stuck in `HALT` after a normal run

`HALT` is terminal — it requires a power cycle by design.  This prevents the
robot from autonomously re-entering `PLAY` without operator confirmation.

Press the physical RESET button or power-cycle the board.

### `HALT` after buffer overflow

```
W (...) path buffer full — halting
I (...) State: RECORD → HALT
```

The recorded path exceeded `PATH_CAPACITY` (512 commands).  Shorten the
recording or increase the capacity:

```rust
// src/domain/path.rs
pub const PATH_CAPACITY: usize = 1024;
```

Note: each `PathCommand` is 4 bytes, so 1024 commands = 4 KB of stack RAM.

### Remote button press is ignored

Button presses are only acted on in these states:
- `IDLE` → start recording
- `RECORD` → finish recording
- `READY` → start playback
- `READY` (empty path) → back to `IDLE`

In `PLAY`, `AVOIDING`, and `HALT`, the button has no effect.

---

## Recovering from an unknown state

If the robot is behaving unexpectedly and you cannot diagnose from logs:

1. Power-cycle the board (full reset to `IDLE`).
2. Reconnect serial monitor.
3. Re-run host unit tests to confirm firmware logic is correct:
   ```bash
   cargo +stable test --lib --target aarch64-apple-darwin
   ```
4. If tests pass, the issue is hardware — check wiring.
5. If tests fail, there is a regression in the domain logic — review recent
   changes to `src/domain/robot.rs`.
