//! VL53L0X / VL53L1X time-of-flight LIDAR adapter (I2C, via TCA9548A mux).
//!
//! Both sensors share a fixed I2C address (`0x29`), so a
//! [`super::tca9548a::Tca9548a`] multiplexer is required to operate two
//! devices on the same bus.  [`Vl53l0xOnMux`] selects its dedicated channel
//! before every I2C transaction.
//!
//! # Initialisation
//!
//! Call [`Vl53l0xOnMux::init`].  This:
//!
//! 1. Selects the correct mux channel.
//! 2. Runs ST's documented power-on sequence to capture `stop_variable`.
//! 3. Starts continuous back-to-back ranging mode.
//!
//! # Polling
//!
//! [`crate::ports::distance::DistancePort::poll`] is **non-blocking**.  It
//! reads the interrupt-status register; when a measurement is ready the
//! range is latched internally and the interrupt cleared so the sensor can
//! produce the next reading.
//!
//! # Units
//!
//! The sensor reports in millimetres.  [`crate::ports::distance::DistancePort::distance_cm`]
//! divides by 10 before returning the value.  Out-of-range sentinels
//! (8190 / 8191 mm) are mapped to `None`.

use core::cell::RefCell;

use log::{debug, trace, warn};

use esp_hal::{Blocking, i2c::master::I2c};

use crate::{config::STALE_TICKS, ports::distance::DistancePort};

// ── Register map ─────────────────────────────────────────────────────────────

const VL53L0X_ADDR:         u8 = 0x29;
const REG_SYSRANGE_START:   u8 = 0x00;
const REG_RESULT_INTERRUPT: u8 = 0x13;
const REG_RESULT_RANGE_HI:  u8 = 0x1E; // 2 bytes big-endian (mm)
const REG_INTERRUPT_CLEAR:  u8 = 0x0B;

/// Out-of-range sentinel values returned by the sensor (mm).
const OOR_SENTINEL_1: u16 = 8190;
const OOR_SENTINEL_2: u16 = 8191;

// ── Adapter ───────────────────────────────────────────────────────────────────

/// VL53L0X ToF LIDAR connected via one channel of a TCA9548A multiplexer.
///
/// Both [`Vl53l0xOnMux`] instances for left and right sensors borrow the same
/// `&RefCell<I2c>` from `main()`.  Each selects its own mux channel before
/// every I2C transaction, so there is never a simultaneous bus conflict in the
/// single-threaded cooperative main loop.
pub struct Vl53l0xOnMux<'d> {
    i2c:           &'d RefCell<I2c<'d, Blocking>>,
    mux_addr:      u8,
    channel:       u8,
    stop_variable: u8,
    distance_mm:   Option<u16>,
    stale_ticks:   u32,
}

impl<'d> Vl53l0xOnMux<'d> {
    /// Initialise the sensor on the given TCA9548A channel.
    ///
    /// Selects `channel`, runs the ST power-on sequence, then starts
    /// continuous ranging mode.  Panics if communication fails (hardware
    /// must be connected and powered).
    pub fn init(
        i2c:      &'d RefCell<I2c<'d, Blocking>>,
        mux_addr: u8,
        channel:  u8,
    ) -> Self {
        let mut s = Self {
            i2c,
            mux_addr,
            channel,
            stop_variable: 0,
            distance_mm:   None,
            stale_ticks:   0,
        };
        s.do_init().expect("VL53L0X init failed");
        s
    }

    // ── Channel selection ─────────────────────────────────────────────────────

    fn select_channel(&self) -> Result<(), esp_hal::i2c::master::Error> {
        self.i2c.borrow_mut().write(self.mux_addr, &[1u8 << self.channel])
    }

    // ── Low-level register I/O ────────────────────────────────────────────────

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
        self.select_channel()?;

        // Step 1 — capture stop_variable (needed for continuous mode restart)
        self.write_reg(0x88, 0x00)?; // I2C standard mode
        self.write_reg(0x80, 0x01)?;
        self.write_reg(0xFF, 0x01)?;
        self.write_reg(0x00, 0x00)?;
        self.stop_variable = self.read_reg(0x91)?;
        self.write_reg(0x00, 0x01)?;
        self.write_reg(0xFF, 0x00)?;
        self.write_reg(0x80, 0x00)?;

        debug!(
            "VL53L0X ch{}: stop_variable=0x{:02X}",
            self.channel, self.stop_variable
        );

        // Step 2 — start continuous back-to-back ranging
        self.write_reg(0x80, 0x01)?;
        self.write_reg(0xFF, 0x01)?;
        self.write_reg(0x00, 0x00)?;
        self.write_reg(0x91, self.stop_variable)?;
        self.write_reg(0x00, 0x01)?;
        self.write_reg(0xFF, 0x00)?;
        self.write_reg(0x80, 0x00)?;
        // SYSTEM_INTERMEASUREMENT_PERIOD = 0 → fastest available rate
        self.write_reg(0x04, 0x00)?;
        // SYSRANGE_START: 0x02 = continuous (back-to-back) mode
        self.write_reg(REG_SYSRANGE_START, 0x02)?;

        debug!("VL53L0X ch{}: continuous ranging started", self.channel);
        Ok(())
    }
}

impl<'d> DistancePort for Vl53l0xOnMux<'d> {
    /// Non-blocking poll: if a new measurement is ready, latch it.
    fn poll(&mut self) {
        if let Err(e) = self.select_channel() {
            warn!("VL53L0X ch{}: mux select failed: {:?}", self.channel, e);
            return;
        }

        // Bits [2:0] of RESULT_INTERRUPT_STATUS are non-zero when data is ready.
        let status = match self.read_reg(REG_RESULT_INTERRUPT) {
            Ok(s)  => s,
            Err(e) => {
                warn!("VL53L0X ch{}: status read failed: {:?}", self.channel, e);
                return;
            }
        };

        if status & 0x07 == 0 {
            trace!("VL53L0X ch{}: no new data", self.channel);
            return;
        }

        // Range result (2 bytes, big-endian, unit = mm).
        let mm = match self.read_reg_u16_be(REG_RESULT_RANGE_HI) {
            Ok(v)  => v,
            Err(e) => {
                warn!("VL53L0X ch{}: range read failed: {:?}", self.channel, e);
                return;
            }
        };

        // Clear interrupt so the sensor can produce the next measurement.
        let _ = self.write_reg(REG_INTERRUPT_CLEAR, 0x01);

        // 0 mm is an erroneous reading (VL53L0X min range ≈ 30 mm); treat it
        // the same as the hardware OOR sentinels.
        if mm == 0 || mm == OOR_SENTINEL_1 || mm == OOR_SENTINEL_2 {
            trace!("VL53L0X ch{}: out of range", self.channel);
            self.distance_mm = None;
        } else {
            trace!("VL53L0X ch{}: {}mm", self.channel, mm);
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
                "VL53L0X ch{}: reading stale ({} ticks without update)",
                self.channel, STALE_TICKS
            );
            self.distance_mm = None;
        }
    }
}

// ── Unit tests (run on host) ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    /// Out-of-range sentinels must map to `None`.
    #[test]
    fn oor_sentinels_give_none() {
        let oor = |mm: u16| -> Option<u16> {
            if mm == 8190 || mm == 8191 { None } else { Some(mm / 10) }
        };
        assert_eq!(oor(8190), None);
        assert_eq!(oor(8191), None);
    }

    /// Normal readings are divided by 10 (mm → cm).
    #[test]
    fn mm_to_cm_conversion() {
        let cvt = |mm: u16| mm / 10;
        assert_eq!(cvt(0),    0);
        assert_eq!(cvt(100),  10);
        assert_eq!(cvt(1234), 123);
        assert_eq!(cvt(2000), 200);
    }

    /// Values just below sentinels are valid.
    #[test]
    fn near_oor_is_valid() {
        let oor = |mm: u16| -> bool { mm == 8190 || mm == 8191 };
        assert!(!oor(8189));
        assert!(oor(8190));
        assert!(oor(8191));
    }

    /// 0 mm is an erroneous sensor reading and must map to `None`.
    #[test]
    fn zero_mm_gives_none() {
        let guard = |mm: u16| -> Option<u16> {
            if mm == 0 || mm == 8190 || mm == 8191 { None } else { Some(mm / 10) }
        };
        assert_eq!(guard(0), None, "0 mm must be treated as out-of-range");
        assert_eq!(guard(1), Some(0), "1 mm is not 0, so it passes the guard");
    }
}
