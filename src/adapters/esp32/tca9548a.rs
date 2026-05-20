//! TCA9548A / PCA9548A 8-channel I2C multiplexer adapter.
//!
//! The mux is controlled by writing a single byte to its I2C address.
//! Each bit selects the corresponding downstream channel (0–7).  Only one
//! channel should be active at a time to prevent address conflicts.
//!
//! # Default address
//!
//! When all address-select pins (A0–A2) are tied to GND the device responds
//! at `0x70` ([`Tca9548a::DEFAULT_ADDR`]).  Tying pins HIGH shifts the address
//! up by 1 per pin, giving a range of `0x70–0x77`.
//!
//! # Bus sharing
//!
//! [`Tca9548a`] borrows `&RefCell<I2c>` so that the same underlying I2C
//! peripheral can be shared with other adapters (e.g. two
//! [`super::vl53l0x::Vl53l0xOnMux`] sensors on different channels).
//! The `RefCell` enforces single-threaded exclusive access at runtime.

use core::cell::RefCell;

use log::debug;

use esp_hal::{Blocking, i2c::master::I2c};

/// Default I2C address when A0–A2 = GND.
pub const DEFAULT_ADDR: u8 = 0x70;

/// TCA9548A / PCA9548A 8-channel I2C bus multiplexer.
pub struct Tca9548a<'d> {
    i2c:  &'d RefCell<I2c<'d, Blocking>>,
    addr: u8,
}

impl<'d> Tca9548a<'d> {
    /// Create the adapter.
    ///
    /// `addr` is typically [`DEFAULT_ADDR`] (0x70) when A0–A2 = GND.
    pub fn new(i2c: &'d RefCell<I2c<'d, Blocking>>, addr: u8) -> Self {
        Self { i2c, addr }
    }

    /// Enable exactly one channel (0–7); all others are disabled.
    ///
    /// Writes `1 << channel` to the mux address.
    pub fn select(&self, channel: u8) -> Result<(), esp_hal::i2c::master::Error> {
        assert!(channel < 8, "TCA9548A channel must be 0–7");
        debug!("TCA9548A: select channel {}", channel);
        self.i2c.borrow_mut().write(self.addr, &[1u8 << channel])
    }

    /// Disable all downstream channels (isolates the upstream bus).
    pub fn disable_all(&self) -> Result<(), esp_hal::i2c::master::Error> {
        debug!("TCA9548A: disable all channels");
        self.i2c.borrow_mut().write(self.addr, &[0x00])
    }

    /// The I2C address this mux instance responds to.
    pub fn address(&self) -> u8 {
        self.addr
    }
}
