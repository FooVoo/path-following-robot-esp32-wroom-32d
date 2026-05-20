//! Hardware pin assignments and tuneable thresholds.
//!
//! GPIO constants are gated per chip architecture:
//!   - `#[cfg(target_arch = "xtensa")]` — ESP32 (WROOM-32D)
//!   - `#[cfg(target_arch = "riscv32")]` — ESP32-C3 (MINI-1 or DevKitM-1)
//!
//! All threshold/timing/address constants are chip-agnostic and shared.
//!
//! # ESP32-WROOM-32D pin notes
//! ┌─────────────────────────────────────────────────────────────┐
//! │ ⚠ GPIO6–11 are normally reserved for the WROOM's quad-SPI  │
//! │   flash.  GPIO9 / GPIO10 are used here for LIDAR_L per the │
//! │   MVP spec – remap if your flash actually uses these pins.  │
//! └─────────────────────────────────────────────────────────────┘
//!
//! # ESP32-C3-MINI-1 pin notes
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ GPIO11–GPIO17  are connected to internal SPI flash and MUST NOT be  │
//! │   used as general-purpose I/O on MINI-1 modules.                    │
//! │ GPIO20 (UART0_TX) / GPIO21 (UART0_RX) are reserved for `esp-        │
//! │   println` debug output.  Do NOT reassign these.                    │
//! │ GPIO18 (USB D−) / GPIO19 (USB D+) are used by the built-in USB      │
//! │   Serial/JTAG peripheral when `jtag-serial` feature is enabled.     │
//! │   This firmware uses the UART0 path instead, so GPIO18/19 are free. │
//! │ GPIO2  ⚠ strapping pin (controls JTAG mode).  The ULN2003 input is  │
//! │   high-impedance at boot so the chip samples VCC via an external     │
//! │   10 kΩ pull-up to 3.3 V.  Required — do NOT omit the pull-up.      │
//! │ GPIO8  ⚠ strapping pin (ROM download log enable).  The I2C external  │
//! │   pull-up holds the line high; no extra resistor needed.             │
//! │ GPIO9  ⚠ strapping pin (BOOT button).  Has an internal weak pull-up  │
//! │   and is sampled high (normal boot) when the ULN2003 input is idle.  │
//! └─────────────────────────────────────────────────────────────────────┘

// ── Motor (DRV8833 H-bridge) ─────────────────────────────────────────────────
/// DRV8833 AIN1 → Motor A forward half-bridge (left wheel forward).
#[cfg(target_arch = "xtensa")]
pub const MOTOR_AIN1_GPIO: u8 = 25;
#[cfg(target_arch = "riscv32")]
pub const MOTOR_AIN1_GPIO: u8 = 3;

/// DRV8833 AIN2 → Motor A reverse half-bridge (left wheel reverse).
#[cfg(target_arch = "xtensa")]
pub const MOTOR_AIN2_GPIO: u8 = 26;
#[cfg(target_arch = "riscv32")]
pub const MOTOR_AIN2_GPIO: u8 = 4;

/// DRV8833 BIN1 → Motor B forward half-bridge (right wheel forward).
#[cfg(target_arch = "xtensa")]
pub const MOTOR_BIN1_GPIO: u8 = 32;
#[cfg(target_arch = "riscv32")]
pub const MOTOR_BIN1_GPIO: u8 = 5;

/// DRV8833 BIN2 → Motor B reverse half-bridge (right wheel reverse).
#[cfg(target_arch = "xtensa")]
pub const MOTOR_BIN2_GPIO: u8 = 33;
#[cfg(target_arch = "riscv32")]
pub const MOTOR_BIN2_GPIO: u8 = 6;

// ── TF-Luna LIDAR UART pins (retained as fallback) ───────────────────────────
/// LIDAR left  – UART1 RX.  ⚠ In WROOM flash range – see module doc.
pub const LIDAR_L_RX_GPIO: u8 = 9;
/// LIDAR left  – UART1 TX (write-only; TF-Luna ignores it for streaming mode).
pub const LIDAR_L_TX_GPIO: u8 = 10;
/// LIDAR right – UART2 RX.
pub const LIDAR_R_RX_GPIO: u8 = 16;
/// LIDAR right – UART2 TX.
pub const LIDAR_R_TX_GPIO: u8 = 17;

// ── I2C bus (shared by TCA9548A + VL53L0X) ───────────────────────────────────
/// I2C SDA.
#[cfg(target_arch = "xtensa")]
pub const I2C_SDA_GPIO: u8 = 21;
/// I2C SDA — ESP32-C3 (GPIO7, safe with I2C external pull-up).
#[cfg(target_arch = "riscv32")]
pub const I2C_SDA_GPIO: u8 = 7;

/// I2C SCL.
#[cfg(target_arch = "xtensa")]
pub const I2C_SCL_GPIO: u8 = 22;
/// I2C SCL — ESP32-C3 (GPIO8 ⚠ strapping, but I2C pull-up holds it high at boot).
#[cfg(target_arch = "riscv32")]
pub const I2C_SCL_GPIO: u8 = 8;

/// I2C bus frequency in Hz (standard mode = 100 kHz).
pub const I2C_FREQ_HZ: u32 = 100_000;

// ── TCA9548A / PCA9548A I2C multiplexer ──────────────────────────────────────
/// TCA9548A 7-bit I2C address (A0–A2 = GND → 0x70).
pub const TCA9548A_ADDR: u8 = 0x70;
/// Multiplexer channel for the **left** VL53L0X.
pub const VL53L0X_LEFT_CHANNEL: u8 = 0;
/// Multiplexer channel for the **right** VL53L0X.
pub const VL53L0X_RIGHT_CHANNEL: u8 = 1;

// ── SSD1306 OLED display ──────────────────────────────────────────────────────
/// SSD1306 7-bit I2C address.
/// Most modules tie SA0 to GND → 0x3C.  Change to 0x3D if SA0 is tied to VCC.
pub const SSD1306_I2C_ADDR: u8 = 0x3C;

// ── LCD 1602 (HD44780, 4-bit parallel) ───────────────────────────────────────
// LCD requires 6 GPIO pins.  On the ESP32-C3-MINI-1 this is not feasible
// without sacrificing UART0 debug or USB Serial/JTAG; the C3 binaries
// therefore use `NoDisplay`.  The constants are kept but only compiled for
// the Xtensa (ESP32) target to avoid unused-constant warnings on C3 builds.
/// LCD register-select (RS) — logic low = command, logic high = data.
#[cfg(target_arch = "xtensa")]
pub const LCD_RS_GPIO: u8 = 5;
/// LCD enable clock (EN) — data latched on falling edge.
#[cfg(target_arch = "xtensa")]
pub const LCD_EN_GPIO: u8 = 4;
/// LCD data bit 4 (D4).
#[cfg(target_arch = "xtensa")]
pub const LCD_D4_GPIO: u8 = 13;
/// LCD data bit 5 (D5).
#[cfg(target_arch = "xtensa")]
pub const LCD_D5_GPIO: u8 = 14;
/// LCD data bit 6 (D6).
#[cfg(target_arch = "xtensa")]
pub const LCD_D6_GPIO: u8 = 15;
/// LCD data bit 7 (D7).
#[cfg(target_arch = "xtensa")]
pub const LCD_D7_GPIO: u8 = 2;

// ── ULN2003 stepper driver (28BYJ-48, half-step) ─────────────────────────────
/// Stepper coil IN1.
/// ⚠ ESP32-C3: GPIO2 is a strapping pin (JTAG mode select).
///   The ULN2003 input is high-impedance during chip reset so the pin floats
///   unless pulled.  Fit a 10 kΩ resistor from GPIO2 to 3.3 V to guarantee
///   "JTAG disabled" sampling; the resistor is overridden by the LEDC driver
///   once the firmware starts.
#[cfg(target_arch = "xtensa")]
pub const STEPPER_IN1_GPIO: u8 = 18;
#[cfg(target_arch = "riscv32")]
pub const STEPPER_IN1_GPIO: u8 = 2;

/// Stepper coil IN2.
/// ⚠ ESP32-C3: GPIO9 is the BOOT strapping pin (active-low boot mode entry).
///   The internal weak pull-up keeps it high under normal conditions and the
///   ULN2003 input presents no load, so no external resistor is needed here.
#[cfg(target_arch = "xtensa")]
pub const STEPPER_IN2_GPIO: u8 = 19;
#[cfg(target_arch = "riscv32")]
pub const STEPPER_IN2_GPIO: u8 = 9;

/// Stepper coil IN3.
#[cfg(target_arch = "xtensa")]
pub const STEPPER_IN3_GPIO: u8 = 23;
#[cfg(target_arch = "riscv32")]
pub const STEPPER_IN3_GPIO: u8 = 18;

/// Stepper coil IN4.
#[cfg(target_arch = "xtensa")]
pub const STEPPER_IN4_GPIO: u8 = 12;
#[cfg(target_arch = "riscv32")]
pub const STEPPER_IN4_GPIO: u8 = 19;

/// Delay between half-steps in microseconds (2 000 µs ≈ 15 rpm shaft speed).
pub const STEPPER_STEP_DELAY_US: u32 = 2_000;

// ── Joystick ──────────────────────────────────────────────────────────────────
/// Joystick X axis – ADC1 channel 0.
/// ESP32: GPIO36 (VP, input-only).  ESP32-C3: GPIO0 (ADC1_CH0).
#[cfg(target_arch = "xtensa")]
pub const JOY_X_GPIO: u8 = 36;
#[cfg(target_arch = "riscv32")]
pub const JOY_X_GPIO: u8 = 0;

/// Joystick Y axis – ADC1.
/// ESP32: GPIO39 / VN (ADC1_CH3, input-only).  ESP32-C3: GPIO1 (ADC1_CH1).
#[cfg(target_arch = "xtensa")]
pub const JOY_Y_GPIO: u8 = 39;
#[cfg(target_arch = "riscv32")]
pub const JOY_Y_GPIO: u8 = 1;

/// Joystick push-button – digital input with internal pull-up, active-low.
///
/// ESP32: GPIO27 (supports internal pull-up on WROOM-32D).
/// ESP32-C3: GPIO10 (supports internal pull-up, no special function).
///
/// ⚠ ESP32 original was GPIO34 which is input-only and has no internal
///   pull-up.  GPIO27 is the recommended assignment — see README.
#[cfg(target_arch = "xtensa")]
pub const JOY_SW_GPIO: u8 = 27;
#[cfg(target_arch = "riscv32")]
pub const JOY_SW_GPIO: u8 = 10;

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
/// Time between joystick samples while in RECORD state (ms).
pub const PATH_CMD_INTERVAL_MS: u64 = 20;

// ── Button debounce ───────────────────────────────────────────────────────────
/// Minimum time between two accepted button-press events (ms).
pub const DEBOUNCE_MS: u64 = 50;
/// Minimum hold time (ms) for a physical button press to be classified as a
/// "long press" and transition IDLE → DIRECT.  Presses shorter than this
/// are classified as short and transition IDLE → RECORD instead.
pub const LONG_PRESS_MS: u64 = 1_000;

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
