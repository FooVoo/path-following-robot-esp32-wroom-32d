# ADR-002 — No Embassy: Bare-Metal WiFi with `esp-wifi` + `smoltcp`

| Field    | Value                                |
|----------|--------------------------------------|
| Date     | 2026-05-17                           |
| Status   | **Accepted**                         |
| Deciders | FooVoo                               |
| Supersedes | *(initial draft used Embassy async)* |

---

## Context

The robot firmware needs WiFi for two purposes:

1. **Telemetry broadcast** — send a JSON UDP datagram every 200 ms containing
   the current FSM state and sensor readings.
2. **Remote control** — receive 4-byte UDP commands to inject throttle values
   or synthetic button presses.

An earlier draft of the firmware pulled in the Embassy async runtime
(`embassy-executor`, `embassy-time`, `embassy-net`) to drive the WiFi stack.
During implementation a series of blockers emerged:

### Blockers with Embassy

| Blocker | Detail |
|---|---|
| **Host tests broken** | `embassy-executor` enables `target_arch = "xtensa"` code paths even when the crate is a dependency of a `--lib` test build on `aarch64`. `cargo test --lib` fails to compile. |
| **`esp` toolchain required for all builds** | Embassy's proc-macro crates assume the `esp` toolchain features.  The host `stable` toolchain cannot build them. |
| **Double executor** | The main loop is a bare `loop {}` — there is no need for task scheduling.  Adding a full async executor for a single WiFi task wastes ~8 KB of flash and ~4 KB of RAM. |
| **Cargo feature explosion** | `embassy-net` pulls in 14+ additional crates, several of which conflict with the no-std smoltcp configuration we need. |
| **`builtin-scheduler` feature gap** | `esp-wifi ≥0.13` with its default `builtin-scheduler` feature requires `esp-hal/__esp_wifi_builtin_scheduler`, which was not present in any published `esp-hal` release up to 1.1.1.  Enabling Embassy defaults therefore broke `cargo check` for any pinned esp-hal version. |

### Goal

Keep the 19-test host unit-test workflow (`cargo +stable test --lib --target
aarch64-apple-darwin`) working unconditionally, while still providing reliable
WiFi on the device.

---

## Decision

**Remove Embassy entirely.  Drive WiFi with `esp-wifi` + `smoltcp` directly,
using a synchronous `block_on` spin-loop for the one-time connect phase.**

### Implementation summary

> **Note (2026-05-19):** `esp-wifi` was renamed to `esp-radio` in version 0.18.
> The earlier draft that described `esp-wifi ~0.14` with `default-features = false`
> to omit `builtin-scheduler` is superseded by the current crate split below.
> The fundamental decision — no Embassy executor, bare-metal WiFi — is unchanged.

```toml
# Cargo.toml
[target.'cfg(target_arch = "xtensa")'.dependencies]
esp-radio          = { version = "0.18", features = ["esp32", "wifi"] }
esp-rtos           = { version = "0.3",  features = ["esp32", "esp-radio"] }
embassy-net-driver = "0.2"          # token traits for smoltcp bridge
esp-alloc          = "0.10"
smoltcp            = { version = "0.12", default-features = false, features = [
    "alloc", "proto-ipv4", "socket-udp", "socket-dhcpv4", "medium-ethernet"] }
```

`esp-radio` (the renamed `esp-wifi`) no longer requires `default-features =
false` to suppress the `builtin-scheduler` issue — the scheduler is now managed
by the separate `esp-rtos` crate.  `embassy-net-driver = "0.2"` is still
required for the `WifiRxToken` / `WifiTxToken` `consume_token` traits used in
the smoltcp device bridge.

**`esp-rtos` is required.**  WiFi ISR tasks run in the background only after
`esp_rtos::start()` is called.  Call it before `WifiAdapter::connect()`:

```rust
// src/bin/main.rs — before WifiAdapter::connect()
let timg1 = TimerGroup::new(peripherals.TIMG1);
let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
esp_rtos::start(timg1.timer0, sw_int.software_interrupt0);
```

### `block_on` — how it works

```rust
fn block_on<T>(mut future: impl Future<Output = T>) -> T {
    let waker = unsafe { Waker::from_raw(RawWaker::new(ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    loop {
        match Pin::new(&mut future).poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending  => core::hint::spin_loop(),
        }
    }
}
```

`esp-wifi`'s futures are driven by hardware interrupts (ISR).  When the ISR
fires, the future becomes `Ready` on the very next `poll()`.  The waker is
a no-op — its only job is to let the future be polled at all.  No task queue,
no heap allocation, no executor thread.

This is safe for the **one-time connect phase** only (associating with the AP
and obtaining the IP lease).  The connect future completes in roughly 2–5 s
and is never polled again.

### Steady-state WiFi operation

After connecting, the WiFi stack is polled explicitly in the main tick loop:

```rust
fn tick(&mut self, ...) {
    // 1. Poll the network interface to drain receive buffers
    self.wifi.poll_network(now_ms);  // calls iface.poll() + drains cmd socket
    // ... FSM logic ...
    // 2. Send telemetry (rate-limited to TELEMETRY_INTERVAL_MS)
    self.wifi.send_telemetry(&frame);
}
```

`iface.poll()` is `smoltcp`'s per-call progress function; it is not async and
does not block.  As long as `tick()` is called at ≥200 Hz the receive buffer
never fills.

### Heap allocation

WiFi requires dynamic allocation for the smoltcp socket descriptors and
DMA receive buffers.  We allocate a **static 72 KB region** dedicated to WiFi:

```rust
// src/bin/main.rs — module level (macro expands at compile time)
esp_alloc::heap_allocator!(size: WIFI_HEAP_SIZE);  // WIFI_HEAP_SIZE = 72 * 1024
```

The `size:` keyword was introduced in `esp-alloc 0.10`; earlier versions used
the positional form `heap_allocator!(WIFI_HEAP_SIZE)`.  The constant is defined
in `src/config.rs` and shared between `main.rs` (which calls the macro) and any
code that needs to know the heap budget.

### `W = NoWifi` — keeping tests green

The WiFi adapter is gated at the module level:

```rust
// src/lib.rs
#[cfg(target_arch = "xtensa")]
pub mod adapters;
```

This means `WifiAdapter` is never seen by the host test build.  The `Robot<M,
L, I, W = NoWifi>` default type parameter ensures all 19 existing tests
continue to compile and pass unchanged on stable Rust for aarch64.

---

## Consequences

**Positive**

- `cargo +stable test --lib --target aarch64-apple-darwin` — 19/19 pass, ~5 s.
- Binary size is reduced by ~40 KB vs the Embassy draft (no executor, no
  `embassy-time` driver, no task-arena allocator).
- Stack usage is deterministic — no hidden executor stacks.
- The WiFi adapter compiles only for Xtensa; it cannot pollute host builds.
- `default-features = false` on `esp-wifi` explicitly declares which features
  are needed, making future upgrades safer.

**Negative / trade-offs**

- The `block_on` spin-loop busy-waits during the ~2–5 s connect phase.
  This is acceptable at startup; the robot does not move until `IDLE` state
  is exited by a button press, which happens after WiFi connects.
- Without an executor, concurrent async tasks cannot be expressed naturally.
  Additional peripherals (e.g. OTA update) would need their own synchronous
  poll loop, not `async fn`.
- `smoltcp` socket management is manual — socket indices must be tracked and
  the interface must be polled at the correct rate.

---

## Alternatives considered

### A: Embassy with `#[cfg(not(test))]` gates

Conditionally exclude Embassy code from host builds.  Tried but failed: the
Embassy proc-macro crates have `build.rs` scripts that run on the host and
expect Xtensa LLVM intrinsics, failing at the expansion stage regardless of
`cfg` gates.

### B: `esp-wifi 0.12` (pre `builtin-scheduler`)

`esp-wifi 0.12` requires `esp-hal ~0.23`, which predates the current API.
Migrating back to the 0.23 HAL would require rewriting all peripheral init
code.  Rejected.

### C: Embassy with `builtin-scheduler` disabled and `stable` toolchain

`esp-wifi ≥0.13` requires `builtin-scheduler` to be in its default features
(which activates `esp-hal/__esp_wifi_builtin_scheduler`).  The flag was only
introduced in an unreleased esp-hal.  Disabling defaults and re-enabling
individual features requires careful bookkeeping and is fragile across
upgrades.  The `default-features = false` approach in this ADR achieves the
same result for our crate without any Embassy dependency.

### D: TCP instead of UDP

TCP would give reliable delivery for the remote-control channel.  The overhead
of connection management (SYN/ACK, retransmit timers) in the spin-loop tick
model is high.  UDP is sufficient for low-latency, best-effort telemetry and
command delivery at 200 ms intervals.  A missed command is a safe outcome (the
robot coasts to a stop); a delayed command is equally undesirable.
