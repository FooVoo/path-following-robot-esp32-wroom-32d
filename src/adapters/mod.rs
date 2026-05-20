//! Hardware adapters — ESP32 production + stub simulation.

pub mod esp32;

/// Stub adapters for the Wokwi simulation binary.
///
/// Compiled only when `--features sim` is active.  These adapters implement
/// the port traits without any HAL dependency, allowing the firmware to build
/// without I2C hardware connected in the simulator.
#[cfg(feature = "sim")]
pub mod stub;
