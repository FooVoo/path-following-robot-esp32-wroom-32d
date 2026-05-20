# ADR-006 — TCA9548A I2C Multiplexer as a Required Hardware Component

| Field      | Value                                                           |
|------------|-----------------------------------------------------------------|
| Status     | Accepted                                                        |
| Deciders   | FooVoo                                                          |
| Date       | 2026-05-19                                                      |
| Supersedes | —                                                               |
| Related    | ADR-001 (hexagonal arch), ADR-005 (I2C bus sharing via RefCell) |

---

## Context

The original design (ADR-001) referenced a single TF-Luna LIDAR for distance sensing.
The hardware was subsequently revised to use **two VL53L0X time-of-flight sensors**
(left and right) for obstacle detection, which introduced a hard address conflict:
both sensors share the fixed I2C address `0x29` and cannot be re-addressed in
software.

### ESP32 WROOM-32D I/O constraints

| Resource          | Available | Allocated                           | Remaining |
|-------------------|-----------|-------------------------------------|-----------|
| I2C peripherals   | 2         | I2C0 (sensors + mux), I2C1 (free)  | 1         |
| GPIO pins (usable)| ~25       | DRV8833 ×2 (4), LCD1602 (6), ULN2003 (4), joystick (3), I2C (2) | ~6 |
| SPI peripherals   | 3         | none allocated                      | 3         |

Connecting two VL53L0X sensors **without** a multiplexer would require either two
separate I2C peripherals (consuming both I2C0 and I2C1, leaving nothing for future
expansion) or a hardware address-select pin (VL53L0X has no `ADDR` pin — the only
supported remapping method is a software sequence that requires individual XSHUT
control during initialisation, consuming one additional GPIO per sensor).

### XSHUT re-addressing alternative

The VL53L0X can be re-addressed at boot time: pull one sensor's XSHUT low, power-cycle
the other, call `set_device_address()`, then release XSHUT.  This reassigns one sensor
to a different address, removing the conflict.

While technically feasible, this approach was evaluated and found unsuitable:

1. **GPIO cost** — requires one dedicated XSHUT GPIO per sensor.  With the current BOM
   (DRV8833 ×2, LCD1602 4-bit, ULN2003, joystick ADC), only ~6 GPIO pins are
   unallocated.  Reserving two for XSHUT eliminates flexibility for future sensors.
2. **Boot-order fragility** — the re-addressing sequence must run before the I2C bus
   is shared with any other driver.  Any reset or power-glitch that skips the sequence
   leaves both sensors at `0x29`, causing silent read failures rather than a clean
   compile-time or init-time error.
3. **No scalability** — the approach breaks down at three or more same-address sensors;
   a multiplexer would still be required at that point.

### Options considered

| Option | Description | Verdict |
|--------|-------------|---------|
| A — TCA9548A/PCA9548A 8-channel mux | Single I2C peripheral, mux selects active channel per transaction. | **Accepted** |
| B — Two separate I2C peripherals | Left sensor on I2C0, right sensor on I2C1; no mux. | Rejected: consumes both hardware I2C blocks, blocking all future I2C expansion. |
| C — XSHUT re-addressing at boot | GPIO per sensor, software re-assign address during init. | Rejected: GPIO-expensive, boot-order fragile, does not scale beyond 2 sensors (see above). |
| D — SPI-capable distance sensor | Replace VL53L0X with a sensor that uses SPI or UART. | Rejected: changes BOM, increases cost, loses existing `Vl53l0xOnMux` adapter. |
| E — Single sensor | One LIDAR, give up stereo detection. | Rejected: stereo left/right obstacle detection is a core functional requirement. |

---

## Decision

The **TCA9548A / PCA9548A 8-channel I2C multiplexer is a required hardware component**,
not an optional upgrade.  It is the only solution that satisfies all three constraints
simultaneously:

1. Resolves the `0x29` address conflict between the two VL53L0X sensors.
2. Consumes a single I2C peripheral (`I2C0`) and no additional GPIO pins (the mux
   sits on the same SDA/SCL lines as the sensors).
3. Leaves I2C1 and the remaining GPIO pins available for future peripherals.

The mux is wired as follows:

```
ESP32 GPIO21 (SDA) ──── TCA9548A SDA ──┬── CH0: VL53L0X left  (addr 0x29)
ESP32 GPIO22 (SCL) ──── TCA9548A SCL  │└── CH1: VL53L0X right (addr 0x29)
                                        └── CH2–CH7: reserved
TCA9548A I2C addr: 0x70 (A0=A1=A2=GND)
```

The firmware selects a channel by writing to the mux control register before each
sensor transaction; only the selected channel's downstream bus is active at any time.

The Rust I2C bus-sharing strategy (how the single `I2c` peripheral handle is shared
between the `Tca9548a` adapter and both `Vl53l0xOnMux` adapters) is documented
separately in **ADR-005**.

---

## Consequences

### Positive

* **GPIO-neutral.** Adding the mux does not consume any additional GPIO pins beyond
  the two already used for I2C (SDA/SCL).
* **Expandable.** Channels CH2–CH7 are available for additional I2C peripherals
  (more sensors, secondary LCD, RTC, etc.) without hardware rewiring.
* **Single I2C peripheral.** I2C1 remains entirely free; SPI and UART buses are
  unaffected.
* **Clean fault isolation.** A NACK from the mux address (`0x70`) at boot is an
  unambiguous wiring fault; no silent address collision.
* **Uniform sensor type.** Both LIDAR adapters are identical `Vl53l0xOnMux` instances,
  preserving the `Robot<M, L, L, I, W>` constraint that requires both to implement
  the same `DistancePort` type (ADR-001).

### Negative / Risks

* **Hard BOM dependency.** The firmware will not boot without the mux physically
  present on the I2C bus; `Tca9548a::select_channel()` issues a write to `0x70` before
  every sensor read, and an I2C NACK panics the adapter.
* **Additional propagation delay.** The mux adds ~10 ns of propagation delay and
  slightly increases capacitive load on the I2C lines.  At 100 kHz this is negligible;
  at 400 kHz (fast mode) pull-up resistor sizing should be verified.
* **Simulator limitation.** `wokwi-tca9548a` does not exist as a native Wokwi
  component.  The simulator uses a `StubDistance` adapter that bypasses the mux
  entirely (see runbook 09-simulation.md and the `sim` Cargo feature).

---

## Implementation notes

* The mux I2C address is configured in `src/config.rs` as `TCA9548A_ADDR = 0x70`.
  If address pins A0–A2 are not all grounded, update this constant.
* Left and right LIDAR channels are `VL53L0X_LEFT_CHANNEL = 0` and
  `VL53L0X_RIGHT_CHANNEL = 1`.  These map directly to bits 0 and 1 of the mux
  control register.
* Channels CH2–CH7 are never written by the current firmware; they remain disabled
  (control register = 0b0000_0000) after each sensor transaction completes.
* The `Tca9548a` struct is a thin helper constructed inline at `main()` and
  immediately consumed by the two `Vl53l0xOnMux::init()` calls; it is not stored
  as a long-lived adapter.
