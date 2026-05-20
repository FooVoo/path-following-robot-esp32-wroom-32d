# ADR-005 — I2C Bus Sharing via `RefCell<I2c>`

| Field   | Value                                                                               |
|---------|-------------------------------------------------------------------------------------|
| Status  | Accepted                                                                            |
| Deciders | FooVoo                                                                              |
| Date    | 2025                                                                                |
| Supersedes | —                                                                                   |
| Related | ADR-001 (hexagonal arch), ADR-002 (no Embassy), ADR-006 (mux as required component) |

---

## Context

The VL53L0X time-of-flight sensor uses a fixed I2C address (`0x29`).  Two
sensors (left and right) are required for obstacle detection.  Two sensors
at the same address cannot coexist on a single I2C bus without a mechanism
to prevent simultaneous access.

A TCA9548A / PCA9548A 8-channel I2C multiplexer resolves the address conflict:
only the selected downstream channel is active at any given time.  However, the
multiplexer and both sensors must still share the **single** `I2c` peripheral
on the ESP32.

The firmware follows ADR-001 (hexagonal architecture), which means adapters are
monomorphised generics — there is no heap, no `Rc`, and no dynamic dispatch.
The solution must also respect ADR-002 (no Embassy / no `async`) — there are no
RTOS tasks, no interrupts competing for the I2C bus.

### Options considered

| Option | Description | Verdict |
|--------|-------------|---------|
| A — Split I2C peripherals | Use I2C0 for left LIDAR and I2C1 for right LIDAR (no mux). | Rejected: ESP32 has only two I2C peripherals; one would be consumed by the TCA9548A itself, leaving none for a second sensor directly. Mux is still needed either way. |
| B — `Rc<RefCell<I2c>>` | Reference-counted shared ownership. | Rejected: requires `alloc`; violates `no_std` without the heap feature. Adds runtime overhead for a single-threaded application. |
| C — `&'d RefCell<I2c<'d, Blocking>>` | Shared borrow of a stack-allocated `RefCell`. | **Accepted** (see below). |
| D — Ownership passed through mux | Mux struct owns `I2c`, sensors access the bus through mux methods only. | Rejected: forces sensors to go through the mux API for every read, coupling their implementation to the mux and making the `DistancePort` trait impure. |
| E — Single aggregate adapter | One struct owns the mux + both sensors; implements `DistancePort` differently for L/R. | Rejected: breaks the existing `Robot<M, L, L, …>` generic constraint that requires both sensors to be the same type. Requires a redesign of the Robot aggregate. |

---

## Decision

Use **`core::cell::RefCell<I2c<'d, Blocking>>`** allocated on the `main()`
stack frame.  Both the `Tca9548a` adapter and each `Vl53l0xOnMux` adapter hold
a `&'d RefCell<I2c<'d, Blocking>>` borrow.

```rust
// src/bin/main.rs (composition root)
let i2c_cell = RefCell::new(I2c::new(peripherals.I2C0, cfg)
    .with_sda(peripherals.GPIO21)
    .with_scl(peripherals.GPIO22));

// Both adapters borrow the same cell:
let lidar_l = Vl53l0xOnMux::init(&i2c_cell, TCA9548A_ADDR, VL53L0X_LEFT_CHANNEL);
let lidar_r = Vl53l0xOnMux::init(&i2c_cell, TCA9548A_ADDR, VL53L0X_RIGHT_CHANNEL);
```

Each adapter calls `i2c_cell.borrow_mut()` for the duration of a single I2C
transaction and immediately drops the borrow.  Because the main loop is purely
sequential (no interrupts touch the I2C bus, no RTOS tasks run concurrently),
`RefCell::borrow_mut()` will never panic at runtime.

---

## Consequences

### Positive

* **No heap.** `RefCell<T>` is `no_std` and stack-allocated.
* **No unsafe code.** `RefCell` enforces the borrow rules at runtime via panic.
* **Clean domain boundary.** The `RefCell` is entirely hidden inside the
  adapter layer.  The `DistancePort` trait and the `Robot` aggregate have no
  knowledge of it.
* **Correct drop order.** `i2c_cell` is declared before the adapters in
  `main()`.  Rust drops locals in reverse declaration order, so `i2c_cell`
  outlives all borrowers.  The lifetime `'d` enforces this at compile time.
* **Zero-cost abstraction.** `RefCell<T>` has no runtime overhead beyond a
  `usize` borrow counter that the compiler can elide in single-borrow paths.

### Negative / Risks

* **Runtime panic on double borrow.** If a future refactor introduces an
  interrupt handler or a second task that calls `borrow_mut()` while the main
  loop holds a borrow, the firmware will panic.  Mitigation: run with
  `#[deny(unsafe_code)]` and keep the I2C bus on a single execution context.
* **Sequential bus access.** Both LIDAR polls are serialised through the same
  physical bus.  At 100 kHz I2C and typical VL53L0X transaction sizes (~10
  bytes), each poll adds ≈ 800 µs.  At 100 Hz loop rate this is well within
  budget.

---

## Implementation notes

* `Tca9548a` is constructed and immediately shadow-dropped (`let _ = mux;`)
  because the mux state is now maintained by each `Vl53l0xOnMux` calling
  `select_channel()` before every I2C transaction.  A future refactor could
  make `Tca9548a` long-lived if channel caching or explicit disable is needed.
* The I2C frequency is 100 kHz (standard mode).  The TCA9548A and VL53L0X are
  both rated up to 400 kHz (fast mode); increasing `I2C_FREQ_HZ` to 400_000
  in `config.rs` is safe if lower latency is needed.
