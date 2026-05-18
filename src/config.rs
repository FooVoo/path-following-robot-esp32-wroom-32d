//! Hardware pin assignments and tuneable thresholds.
//!
//! All GPIO numbers refer to the ESP32-WROOM-32D module.
//!
//! ┌─────────────────────────────────────────────────────────────┐
//! │ ⚠ GPIO6–11 are normally reserved for the WROOM's quad-SPI  │
//! │   flash.  GPIO9 / GPIO10 are used here for LIDAR_L per the │
//! │   MVP spec – remap if your flash actually uses these pins.  │
//! └─────────────────────────────────────────────────────────────┘

// ── Motor (DRV8833 H-bridge) ─────────────────────────────────────────────────
/// DRV8833 AIN1 → Motor A forward half-bridge (left wheel forward).
pub const MOTOR_AIN1_GPIO: u8 = 25;
/// DRV8833 AIN2 → Motor A reverse half-bridge (left wheel reverse).
pub const MOTOR_AIN2_GPIO: u8 = 26;
/// DRV8833 BIN1 → Motor B forward half-bridge (right wheel forward).
pub const MOTOR_BIN1_GPIO: u8 = 32;
/// DRV8833 BIN2 → Motor B reverse half-bridge (right wheel reverse).
pub const MOTOR_BIN2_GPIO: u8 = 33;

// ── TF-Luna LIDAR UART pins ───────────────────────────────────────────────────
/// LIDAR left  – UART1 RX.  ⚠ In WROOM flash range – see module doc.
pub const LIDAR_L_RX_GPIO: u8 = 9;
/// LIDAR left  – UART1 TX (write-only; TF-Luna ignores it for streaming mode).
pub const LIDAR_L_TX_GPIO: u8 = 10;
/// LIDAR right – UART2 RX.
pub const LIDAR_R_RX_GPIO: u8 = 16;
/// LIDAR right – UART2 TX.
pub const LIDAR_R_TX_GPIO: u8 = 17;

// ── Joystick ──────────────────────────────────────────────────────────────────
/// Joystick X axis – ADC1 channel 0 (GPIO36 / VP, input-only pin).
pub const JOY_X_GPIO: u8 = 36;
/// Joystick Y axis – ADC1 channel 3 (GPIO39 / VN, input-only pin).
pub const JOY_Y_GPIO: u8 = 39;
/// Joystick push-button – digital input with internal pull-up, active-low.
///
/// ⚠ GPIO34–39 on the ESP32 are **input-only** and do NOT support internal
///   pull-up or pull-down resistors.  Using GPIO34 here requires an external
///   10 kΩ pull-up resistor to 3.3 V; without it the pin floats and produces
///   phantom presses.
///
/// Recommended: move to GPIO27 (supports internal pull-up, no special function
///   on WROOM-32D) and set `Pull::Up` in the GPIO config.
pub const JOY_SW_GPIO: u8 = 27;

// ── Joystick ADC calibration ─────────────────────────────────────────────────
/// Raw ADC reading at mechanical centre (~1.65 V with 11 dB attenuation).
pub const JOY_CENTER_RAW: u16 = 2048;
/// Raw ADC dead-zone half-width.  Inputs within ±DEAD_ZONE_RAW counts of
/// `JOY_CENTER_RAW` are treated as zero to prevent motor drift at rest.
pub const DEAD_ZONE_RAW: u16 = 100;

// ── LIDAR distance thresholds (centimetres) ───────────────────────────────────
/// Object closer than this triggers obstacle avoidance.
pub const OBSTACLE_CM: u16 = 80;
/// Both sensors must read further than this before resuming playback.
pub const CLEAR_CM: u16 = 100;
/// Advisory warn threshold logged but does not change state.
pub const WARN_CM: u16 = 120;

// ── Avoidance manoeuvre timing ────────────────────────────────────────────────
/// Back-up duration before starting the turn phase (ms).
pub const AVOID_BACK_MS: u64 = 200;
/// Turn duration after backing up (ms).
pub const AVOID_TURN_MS: u64 = 300;
/// If the path is still blocked after this long, transition to HALT (ms).
pub const AVOID_TIMEOUT_MS: u64 = 10_000;

// ── Path recording ────────────────────────────────────────────────────────────
/// Maximum number of `PathCommand` entries stored in the path buffer.
pub const MAX_PATH_CMDS: usize = 512;
/// Time between joystick samples while in RECORD state (ms).
pub const PATH_CMD_INTERVAL_MS: u64 = 20;

// ── Button debounce ───────────────────────────────────────────────────────────
/// Minimum time between two accepted button-press events (ms).
pub const DEBOUNCE_MS: u64 = 50;

// ── ADC ───────────────────────────────────────────────────────────────────────
/// Maximum number of `WouldBlock` retries per `read_oneshot` call before
/// falling back to the joystick centre value (2048).  Prevents the ADC from
/// locking up the cooperative main loop indefinitely.
pub const ADC_MAX_RETRIES: u16 = 200;

// ── LIDAR staleness ───────────────────────────────────────────────────────────
/// Number of main-loop ticks without a valid TF-Luna frame before the reading
/// is discarded.  At 100 Hz loop rate this is 500 ms of no valid frames.
pub const STALE_TICKS: u32 = 50;
/// How often (ms) to repeat the "press button to reset" log message in HALT.
/// Avoids flooding UART at 100 Hz.
pub const HALT_LOG_INTERVAL_MS: u64 = 2_000;
/// PWM carrier frequency for DRV8833 inputs.  1 kHz keeps motor current smooth.
pub const PWM_FREQ_HZ: u32 = 1_000;

// ── Main loop ─────────────────────────────────────────────────────────────────
/// Target loop period (ms).  10 ms gives ~100 Hz state-machine update rate.
pub const LOOP_MS: u64 = 10;

// ── WiFi ──────────────────────────────────────────────────────────────────────
/// WiFi network SSID.
///
/// ⚠ Change to your AP SSID before flashing.
pub const WIFI_SSID: &str = "your_ssid";

/// WiFi WPA2 passphrase.
///
/// ⚠ Change to your AP password before flashing.
pub const WIFI_PASSWORD: &str = "your_password";

/// UDP port the robot listens on for 4-byte remote-control packets:
/// `[0xA5, type, v1, v2]`.
/// * `type = 0x01` — throttle: `v1 = left as u8`, `v2 = right as u8`
/// * `type = 0x02` — button press (v1, v2 ignored)
pub const WIFI_CMD_PORT: u16 = 9000;

/// UDP port the robot broadcasts ~100-byte JSON telemetry frames to.
///
/// The host-side `telemetry-server` binary listens on this same port by
/// default (overridable via the `TELEMETRY_UDP_PORT` env var).
pub const WIFI_TEL_PORT: u16 = 9001;

/// How often to emit a telemetry UDP packet (ms).  200 ms = 5 Hz.
pub const TELEMETRY_INTERVAL_MS: u64 = 200;

/// Maximum time (ms) the robot waits for a DHCP lease before giving up and
/// operating in offline mode (no remote control, no telemetry).
pub const WIFI_DHCP_TIMEOUT_MS: u64 = 15_000;

/// Heap size (bytes) reserved for the WiFi firmware and smoltcp buffers.
///
/// The `esp_alloc::heap_allocator!()` call in `main.rs` uses this constant.
pub const WIFI_HEAP_SIZE: usize = 72 * 1024;
