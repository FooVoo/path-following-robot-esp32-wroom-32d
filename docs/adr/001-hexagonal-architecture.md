# ADR-001 — Hexagonal Architecture

| Field    | Value        |
|----------|--------------|
| Date     | 2026-05-17   |
| Status   | **Accepted** |
| Deciders | FooVoo       |

---

## Context

The firmware controls physical hardware (DRV8833 H-bridge, TF-Luna LIDAR ×2,
analog joystick, WiFi chip) through `esp-hal`, which:

- Panics at compile time when the target is not Xtensa
- Requires a real ESP32 to run any integration test
- Changes its public API between minor releases

The FSM that drives the robot has **non-trivial logic** (six states, obstacle
avoidance, timeout handling, path replay) that must be verified in CI without
physical hardware and without cross-compilation.

Additionally, the project may eventually support other microcontrollers (e.g.
Raspberry Pi Pico, STM32), which would require swapping hardware drivers while
keeping the domain behaviour unchanged.

---

## Decision

We structure the firmware using a **ports-and-adapters (hexagonal) architecture**
split by hardware type:

```
src/
├── domain/          # Pure Rust — no esp-hal, no alloc, no std
│   ├── state.rs
│   ├── path.rs
│   └── robot.rs     # Robot<M, L, I, W, D> — all generics, no concrete types
├── ports/           # Trait definitions — the "hexagon's edge"
│   ├── motors.rs
│   ├── distance.rs
│   ├── input.rs
│   ├── remote_control.rs
│   ├── telemetry.rs
│   ├── display.rs   # DisplayPort trait (added with SSD1306 / LCD1602 support)
│   └── stepper.rs   # StepperPort trait (added with ULN2003 stepper support)
└── adapters/
    └── esp32/       # Concrete esp-hal bindings — xtensa + riscv32 gated
        ├── drv8833.rs
        ├── tf_luna.rs         # TF-Luna UART LIDAR (legacy / fallback)
        ├── vl53l0x.rs         # VL53L0X on TCA9548A mux (primary LIDAR)
        ├── vl53l0x_direct.rs  # VL53L0X on direct I2C bus (single-sensor builds)
        ├── tca9548a.rs        # TCA9548A I2C multiplexer driver
        ├── lcd1602.rs         # LCD1602 character display over I2C
        ├── ssd1306_oled.rs    # SSD1306 OLED display over I2C
        ├── uln2003.rs         # ULN2003 stepper motor driver
        ├── joystick.rs
        └── wifi.rs
```

### Key rules

1. **The domain imports nothing from `esp-hal`.**  The entire `adapters/esp32/`
   subtree is gated with `#[cfg(any(target_arch = "xtensa", target_arch = "riscv32"))]`
   in `src/lib.rs`, keeping both the ESP32 (Xtensa) and ESP32-C3/S3 (RISC-V) targets
   supported while still producing a host-testable crate.

2. **Ports are traits only.**  A port trait has no associated data, no default
   implementations, and no hardware-specific types in its signature.

3. **Adapters own the peripherals.**  Each adapter struct holds the `esp-hal`
   peripheral handle.  The domain receives a `&mut dyn MotorPort` (or a
   concrete generic), never an `Ledc` handle.

4. **The composition root (`src/bin/main.rs`) is the only place** where
   concrete adapter types appear.  It constructs adapters, injects them into
   `Robot::new_with_wifi()`, then calls `robot.tick()` in a loop.

5. **Tests live in the domain and use `Mock*` types**, never hardware.
   Mock implementations reside in `#[cfg(test)]` blocks inside `robot.rs`.

### Generics vs `dyn`

The `Robot` struct is generic over its port implementations:

```rust
pub struct Robot<M, L, I, W = NoWifi, D = NoDisplay>
where
    M: MotorPort,
    L: DistancePort,
    I: InputPort,
    W: RemoteControlPort + TelemetryPort,
    D: DisplayPort,
{ ... }
```

Generics were chosen over trait objects (`dyn Trait`) because:

- Zero-cost abstraction — the compiler monomorphises a single concrete type for
  the whole firmware; no vtable dispatch in the hot tick loop.
- Easier to satisfy `no_std` lifetime constraints — `dyn Trait + 'static`
  would require boxing, which needs `alloc`.
- The firmware only ever constructs one `Robot` instance, so code-size
  duplication from monomorphisation is not a concern.

### `W = NoWifi`, `D = NoDisplay` defaults

Existing test code uses the three-generic form
`Robot<MockMotors, MockDistance, MockInput>`.  Adding `W` and `D` as generic
parameters with `NoWifi` / `NoDisplay` zero-sized-type defaults means all 88
tests continue to compile and pass without any test-file changes.

```rust
/// Zero-sized types that implement their ports as silent no-ops.
pub struct NoWifi;
pub struct NoDisplay;

impl RemoteControlPort for NoWifi { /* all methods return None / false */ }
impl TelemetryPort    for NoWifi { fn send(&mut self, _: &TelemetryFrame) {} }
impl DisplayPort      for NoDisplay { /* all methods are no-ops */ }
```

---

## Consequences

**Positive**

- `cargo +stable test --lib --target aarch64-apple-darwin` runs 88 tests in
  ~5 seconds without a connected ESP32.
- Adding a new hardware driver requires only:
  1. Implement the relevant port trait in a new file under `adapters/esp32/`
  2. Wire it up in `main.rs`
  3. The domain is untouched.
- Porting to a different MCU means writing new adapter files and a new
  `main.rs`; the domain and ports are reused verbatim.
- Defects in avoidance logic, state transitions, and path replay can be
  reproduced as unit tests without any hardware.

**Negative / trade-offs**

- Boilerplate: a new hardware sensor requires both a port trait and an adapter
  struct, even for simple cases.
- The five-generic `Robot<M, L, I, W, D>` signature is verbose in `main.rs`;
  the compiler message for a missing trait bound can be long.
- `block_on` spin-loops (used in the WiFi adapter) are harder to test than
  async code; this is a deliberate choice — see ADR-002.

---

## Alternatives considered

### A: Monolithic `main.rs` with inline esp-hal calls

Simple, but makes the FSM logic untestable on the host.  One wrong threshold
constant could only be caught by flashing and observing behaviour.  Rejected.

### B: Embassy async executor throughout

Would allow `async fn tick()` and `async` adapters.  However, Embassy adds a
large dependency tree and requires the `esp` toolchain for all builds including
`cargo test --lib`.  This prevents the host unit-test workflow.  See ADR-002.

### C: `dyn Trait` instead of generics

Would allow storing `Box<dyn MotorPort>` and simplify the `Robot` signature.
Requires `alloc`, which in turn requires the heap allocator to be initialised
before the robot is constructed.  The heap is currently reserved for WiFi only.
Rejected to avoid accidental heap use in the domain.
