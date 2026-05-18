# Runbook 07 — Development Guide

> **Audience:** Engineers adding new features, new sensors, or porting to a new MCU.

---

## 1  Development workflow

```bash
# 1. Make changes to domain or ports
# 2. Run host tests immediately — no hardware needed
cargo +stable test --lib --target aarch64-apple-darwin

# 3. Check xtensa build compiles (no flash needed)
cargo +esp check

# 4. Flash only when domain tests pass and xtensa check is clean
cargo +esp build --release
espflash flash --monitor target/xtensa-esp32-none-elf/release/path-following-robot
```

Keep this cycle tight.  The host test loop is ~5 seconds; the flash cycle is
~30 seconds.  Defer flashing until logic is proven.

---

## 2  Adding a new sensor

### Step 1 — Define a port trait

Create `src/ports/my_sensor.rs`:

```rust
/// Port for a hypothetical soil-moisture sensor.
pub trait SoilMoisturePort {
    /// Returns moisture percentage [0, 100], or None if sensor is unresponsive.
    fn read_moisture(&mut self) -> Option<u8>;
}
```

Add `pub mod my_sensor;` to `src/ports/mod.rs`.

### Step 2 — Implement the adapter

Create `src/adapters/esp32/my_sensor.rs`:

```rust
// This file is only compiled for xtensa (gated in lib.rs).
use esp_hal::i2c::I2c;
use crate::ports::my_sensor::SoilMoisturePort;

pub struct MySoilSensor<'a> {
    i2c: I2c<'a, esp_hal::Blocking>,
}

impl<'a> MySoilSensor<'a> {
    pub fn new(i2c: I2c<'a, esp_hal::Blocking>) -> Self { Self { i2c } }
}

impl<'a> SoilMoisturePort for MySoilSensor<'a> {
    fn read_moisture(&mut self) -> Option<u8> {
        // ... read I2C register ...
        Some(42)
    }
}
```

Add `pub mod my_sensor;` to `src/adapters/esp32/mod.rs`.

### Step 3 — Add a mock for tests

In `src/domain/robot.rs`, inside `#[cfg(test)]`:

```rust
struct MockSoilSensor(u8);
impl SoilMoisturePort for MockSoilSensor {
    fn read_moisture(&mut self) -> Option<u8> { Some(self.0) }
}
```

### Step 4 — Thread through the domain

Add a fifth generic to `Robot`:

```rust
pub struct Robot<M, L, I, W = NoWifi, S = NoSoil>
where
    S: SoilMoisturePort,
{ ... }
```

Use the same `NoSoil` zero-sized default pattern as `NoWifi` to keep existing
tests compiling unchanged.

### Step 5 — Wire up in `main.rs`

```rust
let soil = MySoilSensor::new(I2c::new(peripherals.I2C0, ...));
let mut robot = Robot::new_with_soil(motors, lidar_l, lidar_r, joystick, wifi, soil);
```

---

## 3  Adding a new FSM state

### Step 1 — Add the variant

`src/domain/state.rs`:

```rust
pub enum RobotState {
    // existing variants ...
    /// New state example.
    Charging,
}

impl RobotState {
    pub fn name(self) -> &'static str {
        match self {
            // existing arms ...
            Self::Charging => "CHARGING",
        }
    }
}
```

### Step 2 — Add a tick handler

`src/domain/robot.rs`:

```rust
fn tick_charging(&mut self, now_ms: u64) {
    // implement state logic
    // use self.motors, self.lidar_l, self.lidar_r, self.input
    // transition with:  self.state = RobotState::Idle;
}
```

### Step 3 — Call it from `tick()`

```rust
pub fn tick(&mut self, now_ms: u64) {
    match self.state {
        // existing arms ...
        RobotState::Charging => self.tick_charging(now_ms),
    }
}
```

### Step 4 — Write tests

```rust
#[test]
fn charging_returns_to_idle_when_full() {
    let mut robot = make_robot();
    robot.state = RobotState::Charging;
    // drive conditions
    robot.tick(0);
    assert_eq!(robot.state, RobotState::Idle);
}
```

---

## 4  Porting to a different MCU

The domain and ports are MCU-agnostic.  Only the adapter layer and `main.rs`
need to change.

### Step 1 — Create a new adapter directory

```
src/adapters/
    esp32/          ← existing
    rp2040/         ← new port example
        mod.rs
        motor.rs    ← implements MotorPort using RP2040 PWM
        lidar.rs    ← implements DistancePort using RP2040 UART
        joystick.rs
        wifi.rs     ← or NoWifi if the MCU has no WiFi
```

### Step 2 — Gate adapters in `lib.rs`

```rust
// src/lib.rs
#[cfg(target_arch = "xtensa")]
pub mod adapters { pub mod esp32; }

// Add the new target:
#[cfg(target_arch = "arm")]
pub mod adapters { pub mod rp2040; }
```

### Step 3 — Write a new `main.rs` (or feature-flag the existing one)

The RP2040 main would:
1. Import `rp2040_hal` instead of `esp_hal`
2. Construct `Rp2040Motor`, `Rp2040Lidar`, etc.
3. Call `Robot::new()` with the same pattern

The domain, ports, and tests require **zero changes**.

---

## 5  Extending the WiFi protocol

### Adding a new telemetry field

1. Add the field to `TelemetryFrame` in `src/ports/telemetry.rs`
2. Populate it in `Robot::tick()` in `src/domain/robot.rs`
3. Serialise it in `WifiAdapter::send_telemetry()` in `src/adapters/esp32/wifi.rs`

Example — adding battery voltage:

```rust
// ports/telemetry.rs
pub struct TelemetryFrame {
    // existing fields ...
    /// Battery voltage in millivolts, or 0 if unavailable.
    pub battery_mv: u16,
}
```

```rust
// adapters/esp32/wifi.rs — inside format_telemetry()
write!(buf, r#","bat":{}"#, frame.battery_mv).ok();
```

### Adding a new command type

1. Pick an unused `type` byte (currently `0x03` and above are free).
2. Add a branch in `WifiAdapter::parse_command()`.
3. Add a corresponding method to `RemoteControlPort` in `src/ports/remote_control.rs`.
4. Consume the command in `Robot::tick()`.

---

## 6  Code style

- **No `unsafe` in the domain layer.**  Any unsafe must live in an adapter and
  be documented with a `// SAFETY:` comment.
- **No `unwrap()` on hardware operations** — use `expect("short reason")` so
  panics are identifiable in the log.
- **Log levels:**
  - `error!` — unrecoverable hardware failure
  - `warn!`  — degraded operation (stale sensor, WiFi disconnected)
  - `info!`  — state transitions, boot milestones
  - `debug!` — per-tick sensor reads, command receipt
  - `trace!` — raw byte values, ADC counts
- **Prefer `#[inline]`** on hot-path functions in adapters (`set_duty`, `parse_frame`).
- **Run Clippy before committing:**
  ```bash
  cargo +esp clippy -- -D warnings
  cargo +stable clippy --lib --target aarch64-apple-darwin -- -D warnings
  ```

---

## 7  Adding new unit tests

All tests live in `src/domain/robot.rs` under `#[cfg(test)]`.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // ── Mock implementations (zero-sized, deterministic) ────────────────────
    struct MockMotors { last_l: i8, last_r: i8 }
    impl MotorPort for MockMotors {
        fn set_throttle(&mut self, l: i8, r: i8) { self.last_l = l; self.last_r = r; }
        fn coast(&mut self) { self.last_l = 0; self.last_r = 0; }
    }

    // Helper — build a robot in a known initial state
    fn make_robot() -> Robot<MockMotors, MockDistance, MockInput> {
        Robot::new(
            MockMotors { last_l: 0, last_r: 0 },
            MockDistance(None),
            MockInput { button: false, throttle: (0, 0) },
        )
    }

    #[test]
    fn my_new_test() {
        let mut robot = make_robot();
        // arrange
        // act
        robot.tick(0);
        // assert
        assert_eq!(robot.state(), RobotState::Idle);
    }
}
```

Tests must be:
- **Deterministic** — no random state, no timing dependencies.
- **Isolated** — one behaviour per test, one assertion per test where practical.
- **Named by behaviour** — `<state>_<condition>_<expected_outcome>`.

---

## 8  Dependency upgrade procedure

```bash
# 1. Check what can be updated
cargo +esp outdated

# 2. For esp-hal specifically — check the changelog for API breaks:
#    https://github.com/esp-rs/esp-hal/blob/main/esp-hal/CHANGELOG.md

# 3. Update version in Cargo.toml

# 4. Run host tests
cargo +stable test --lib --target aarch64-apple-darwin

# 5. Run xtensa check
cargo +esp check

# 6. Flash and smoke-test on hardware
```

> ⚠ **`esp-hal` and `esp-wifi` must be upgraded together** — they share
> internal feature flags.  Check the compatibility matrix in
> `esp-wifi`'s README before upgrading either.
