//! VL53L0X / VL53L1X time-of-flight LIDAR adapter — direct I2C, no mux.
//!
//! Use when only a **single** VL53L0X is connected on the I2C bus at its
//! fixed address (`0x29`).  No TCA9548A multiplexer is required.
//!
//! This adapter is intended for the local `dev` build variant (`--features dev`).
//! For the production firmware with two sensors on the same bus, see
//! [`super::vl53l0x::Vl53l0xOnMux`] which selects a TCA9548A channel before
//! every I2C transaction.
//!
//! # Polling
//!
//! [`crate::ports::distance::DistancePort::poll`] is non-blocking.  It reads the
//! interrupt-status register; when a measurement is ready the range is latched
//! and the interrupt cleared.
//!
//! # Units
//!
//! The sensor reports in millimetres; [`DistancePort::distance_cm`] divides by 10.
//! Out-of-range sentinels (8190 / 8191 mm) are mapped to `None`.

use core::cell::RefCell;

use log::{debug, trace, warn};

use esp_hal::{Blocking, i2c::master::I2c};

use crate::{config::STALE_TICKS, ports::distance::DistancePort};

// ── Register map ─────────────────────────────────────────────────────────────

const VL53L0X_ADDR:         u8 = 0x29;
const REG_SYSRANGE_START:   u8 = 0x00;
const REG_RESULT_INTERRUPT: u8 = 0x13;
const REG_RESULT_RANGE_HI:  u8 = 0x1E; // 2 bytes, big-endian (mm)
const REG_INTERRUPT_CLEAR:  u8 = 0x0B;

const OOR_SENTINEL_1: u16 = 8190;
const OOR_SENTINEL_2: u16 = 8191;

// ── Adapter ───────────────────────────────────────────────────────────────────

/// VL53L0X ToF LIDAR connected directly on the I2C bus (no multiplexer).
///
/// Borrows the shared `&RefCell<I2c>` from `main()`; see ADR-005 for the
/// bus-sharing rationale.  When only one sensor is present there is no address
/// conflict and no channel selection is needed.
pub struct Vl53l0xDirect<'d> {
    i2c:           &'d RefCell<I2c<'d, Blocking>>,
    stop_variable: u8,
    distance_mm:   Option<u16>,
    stale_ticks:   u32,
}

impl<'d> Vl53l0xDirect<'d> {
    /// Initialise the sensor: run the ST power-on sequence then start
    /// continuous back-to-back ranging.
    ///
    /// Panics if I2C communication fails (sensor must be powered and wired).
    pub fn init(i2c: &'d RefCell<I2c<'d, Blocking>>) -> Self {
        let mut s = Self {
            i2c,
            stop_variable: 0,
            distance_mm:   None,
            stale_ticks:   0,
        };
        s.do_init().expect("Vl53l0xDirect init failed — check I2C wiring");
        s
    }

    // ── Register I/O ─────────────────────────────────────────────────────────

    fn write_reg(&self, reg: u8, val: u8) -> Result<(), esp_hal::i2c::master::Error> {
        self.i2c.borrow_mut().write(VL53L0X_ADDR, &[reg, val])
    }

    fn read_reg(&self, reg: u8) -> Result<u8, esp_hal::i2c::master::Error> {
        let mut buf = [0u8; 1];
        self.i2c.borrow_mut().write_read(VL53L0X_ADDR, &[reg], &mut buf)?;
        Ok(buf[0])
    }

    fn read_reg_u16_be(&self, reg: u8) -> Result<u16, esp_hal::i2c::master::Error> {
        let mut buf = [0u8; 2];
        self.i2c.borrow_mut().write_read(VL53L0X_ADDR, &[reg], &mut buf)?;
        Ok(u16::from_be_bytes(buf))
    }

    // ── Initialisation sequence ───────────────────────────────────────────────
    //
    // Derived from ST UM2039 application note § 7 (minimal working sequence).

    fn do_init(&mut self) -> Result<(), esp_hal::i2c::master::Error> {
        // Capture stop_variable (needed to restart continuous mode after ranging).
        self.write_reg(0x88, 0x00)?; // I2C standard mode
        self.write_reg(0x80, 0x01)?;
        self.write_reg(0xFF, 0x01)?;
        self.write_reg(0x00, 0x00)?;
        self.stop_variable = self.read_reg(0x91)?;
        self.write_reg(0x00, 0x01)?;
        self.write_reg(0xFF, 0x00)?;
        self.write_reg(0x80, 0x00)?;

        debug!("Vl53l0xDirect: stop_variable=0x{:02X}", self.stop_variable);

        // Start continuous back-to-back ranging.
        self.write_reg(0x80, 0x01)?;
        self.write_reg(0xFF, 0x01)?;
        self.write_reg(0x00, 0x00)?;
        self.write_reg(0x91, self.stop_variable)?;
        self.write_reg(0x00, 0x01)?;
        self.write_reg(0xFF, 0x00)?;
        self.write_reg(0x80, 0x00)?;
        self.write_reg(0x04, 0x00)?; // SYSTEM_INTERMEASUREMENT_PERIOD = 0 (fastest)
        self.write_reg(REG_SYSRANGE_START, 0x02)?; // 0x02 = continuous mode

        debug!("Vl53l0xDirect: continuous ranging started");
        Ok(())
    }
}

impl<'d> DistancePort for Vl53l0xDirect<'d> {
    fn poll(&mut self) {
        // Bits [2:0] of RESULT_INTERRUPT_STATUS are non-zero when data is ready.
        let status = match self.read_reg(REG_RESULT_INTERRUPT) {
            Ok(s)  => s,
            Err(e) => {
                warn!("Vl53l0xDirect: status read failed: {:?}", e);
                return;
            }
        };

        if status & 0x07 == 0 {
            trace!("Vl53l0xDirect: no new data");
            return;
        }

        let mm = match self.read_reg_u16_be(REG_RESULT_RANGE_HI) {
            Ok(v)  => v,
            Err(e) => {
                warn!("Vl53l0xDirect: range read failed: {:?}", e);
                return;
            }
        };

        // Clear interrupt so the sensor queues the next measurement.
        let _ = self.write_reg(REG_INTERRUPT_CLEAR, 0x01);

        // 0 mm is an erroneous reading (VL53L0X min range ≈ 30 mm); treat it
        // the same as the hardware OOR sentinels.
        if mm == 0 || mm == OOR_SENTINEL_1 || mm == OOR_SENTINEL_2 {
            trace!("Vl53l0xDirect: out of range");
            self.distance_mm = None;
        } else {
            trace!("Vl53l0xDirect: {}mm", mm);
            self.distance_mm = Some(mm);
        }
        self.stale_ticks = 0;
    }

    fn distance_cm(&self) -> Option<u16> {
        self.distance_mm.map(|mm| mm / 10)
    }

    fn tick_staleness(&mut self) {
        self.stale_ticks = self.stale_ticks.saturating_add(1);
        if self.stale_ticks == STALE_TICKS {
            debug!(
                "Vl53l0xDirect: reading stale ({} ticks without update)",
                STALE_TICKS
            );
            self.distance_mm = None;
        }
    }
}

// ── Unit tests (run on host) ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{OOR_SENTINEL_1, OOR_SENTINEL_2};

    fn filter(mm: u16) -> Option<u16> {
        if mm == 0 || mm == OOR_SENTINEL_1 || mm == OOR_SENTINEL_2 {
            None
        } else {
            Some(mm / 10)
        }
    }

    /// 0 mm is an erroneous reading; must map to `None`.
    #[test]
    fn zero_mm_gives_none() {
        assert_eq!(filter(0), None, "0 mm must be treated as out-of-range");
    }

    /// Both hardware OOR sentinels must map to `None`.
    #[test]
    fn oor_sentinels_give_none() {
        assert_eq!(filter(OOR_SENTINEL_1), None);
        assert_eq!(filter(OOR_SENTINEL_2), None);
    }

    /// Normal readings are divided by 10 (mm → cm).
    #[test]
    fn mm_to_cm_conversion() {
        assert_eq!(filter(100),  Some(10));
        assert_eq!(filter(1234), Some(123));
        assert_eq!(filter(500),  Some(50));
    }

    /// Values just below the OOR sentinels are valid (not filtered).
    #[test]
    fn near_oor_is_valid() {
        assert_eq!(filter(8189), Some(818));
        assert_ne!(filter(8189), None);
    }
}
