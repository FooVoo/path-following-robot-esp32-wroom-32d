//! ESP32-specific hardware adapters.

pub mod drv8833;
pub mod joystick;
pub mod lcd1602;
pub mod ssd1306_oled;
pub mod tca9548a;
pub mod tf_luna;
pub mod uln2003;
pub mod vl53l0x;
pub mod vl53l0x_direct;

/// WiFi adapter — excluded from simulation, dev, and C3-dev builds.
/// Those builds use `NoWifi`; `wifi.rs` depends on static config and hardware
/// WiFi peripheral setup that is not needed (and slows compilation) in those modes.
#[cfg(not(any(feature = "sim", feature = "dev", feature = "esp32c3-dev")))]
pub mod wifi;

