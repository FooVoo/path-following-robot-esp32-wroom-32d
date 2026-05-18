//! TF-Luna LIDAR adapter.
//!
//! The TF-Luna communicates over UART at 115 200 baud.  It continuously
//! emits 9-byte frames at up to 100 Hz:
//!
//! ```text
//! Byte  0: 0x59  (header 1)
//! Byte  1: 0x59  (header 2)
//! Byte  2: dist_low
//! Byte  3: dist_high   → distance (cm) = (dist_high << 8) | dist_low
//! Byte  4: strength_low
//! Byte  5: strength_high
//! Byte  6: temp_low
//! Byte  7: temp_high
//! Byte  8: checksum    = (bytes 0..8).sum() & 0xFF
//! ```
//!
//! The parser is byte-driven (no heap allocation) and handles arbitrary
//! UART chunking.
//!
//! # Staleness
//!
//! After `STALE_TICKS` ticks without a valid frame the reading is discarded.
//! This prevents the robot from acting on readings that are no longer current
//! (e.g. sensor disconnected mid-flight).

use log::{debug, trace, warn};

use esp_hal::{uart::UartRx, Blocking};

use crate::{config::STALE_TICKS, ports::distance::DistancePort};

// ---------------------------------------------------------------------------
// Low-level frame parser (no HAL dependency)
// ---------------------------------------------------------------------------

const HEADER: u8 = 0x59;
const FRAME_LEN: usize = 9;

/// Incremental TF-Luna frame parser.
///
/// Feed bytes one-at-a-time via [`TfLunaParser::push`].  Completed frames
/// are automatically validated and the distance is stored.
#[derive(Default)]
pub struct TfLunaParser {
    buf: [u8; FRAME_LEN],
    pos: usize,
    /// Most recent valid distance reading (cm).
    distance_cm: Option<u16>,
    /// Ticks since the last valid frame.  Capped at `u32::MAX`.
    ticks_since_update: u32,
}

impl TfLunaParser {
    /// Feed one byte into the parser.
    pub fn push(&mut self, b: u8) {
        if self.pos == 0 {
            // Synchronise on the first header byte.
            if b == HEADER {
                self.buf[0] = b;
                self.pos = 1;
            }
        } else if self.pos == 1 {
            // Synchronise on the second header byte.
            if b == HEADER {
                self.buf[1] = b;
                self.pos = 2;
            } else {
                // Not a valid second header; restart sync.
                self.pos = 0;
            }
        } else {
            self.buf[self.pos] = b;
            self.pos += 1;

            if self.pos == FRAME_LEN {
                self.pos = 0;
                self.try_commit();
            }
        }
    }

    /// Validate the completed frame and, on success, update `distance_cm`.
    fn try_commit(&mut self) {
        let checksum: u8 = self.buf[..8].iter().fold(0u8, |acc, &x| acc.wrapping_add(x));
        if checksum != self.buf[8] {
            warn!(
                "TF-Luna checksum mismatch: expected {:#04x} got {:#04x}",
                checksum, self.buf[8]
            );
            return;
        }
        let dist = u16::from_le_bytes([self.buf[2], self.buf[3]]);
        trace!("TF-Luna frame ok: {}cm", dist);
        self.distance_cm = Some(dist);
        self.ticks_since_update = 0;
    }

    /// Advance the staleness counter.  Call once per main-loop tick.
    pub fn tick(&mut self) {
        self.ticks_since_update = self.ticks_since_update.saturating_add(1);
        if self.ticks_since_update == STALE_TICKS {
            debug!("TF-Luna reading stale ({}+ ticks)", STALE_TICKS);
            self.distance_cm = None;
        }
    }

    /// Most recent valid distance (cm), or `None` if stale/unavailable.
    pub fn distance_cm(&self) -> Option<u16> {
        self.distance_cm
    }
}

// ---------------------------------------------------------------------------
// DistancePort adapter
// ---------------------------------------------------------------------------

/// ESP32 TF-Luna adapter: wraps a blocking [`UartRx`] and a [`TfLunaParser`].
pub struct TfLuna<'d> {
    rx: UartRx<'d, Blocking>,
    parser: TfLunaParser,
}

impl<'d> TfLuna<'d> {
    /// Create the adapter from a configured `UartRx`.
    pub fn new(rx: UartRx<'d, Blocking>) -> Self {
        Self {
            rx,
            parser: TfLunaParser::default(),
        }
    }
}

impl<'d> DistancePort for TfLuna<'d> {
    /// Drain all available UART bytes into the frame parser.
    fn poll(&mut self) {
        while self.rx.read_ready() {
            let mut byte = [0u8; 1];
            match self.rx.read(&mut byte) {
                Ok(1) => self.parser.push(byte[0]),
                Ok(_) => {}
                Err(e) => {
                    warn!("TF-Luna UART read error: {:?}", e);
                    break;
                }
            }
        }
    }

    fn distance_cm(&self) -> Option<u16> {
        self.parser.distance_cm()
    }

    fn tick_staleness(&mut self) {
        self.parser.tick();
    }
}

// ---------------------------------------------------------------------------
// Unit tests (run on host)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::TfLunaParser;

    /// Build a valid 9-byte TF-Luna frame for the given distance.
    fn make_frame(dist_cm: u16) -> [u8; 9] {
        let dl = (dist_cm & 0xFF) as u8;
        let dh = (dist_cm >> 8) as u8;
        let mut f = [0x59, 0x59, dl, dh, 0x00, 0x00, 0x00, 0x00, 0x00];
        f[8] = f[..8].iter().fold(0u8, |a, &x| a.wrapping_add(x));
        f
    }

    #[test]
    fn parses_single_frame() {
        let mut p = TfLunaParser::default();
        for &b in make_frame(150).iter() {
            p.push(b);
        }
        assert_eq!(p.distance_cm(), Some(150));
    }

    #[test]
    fn bad_checksum_discarded() {
        let mut frame = make_frame(200);
        frame[8] = frame[8].wrapping_add(1); // corrupt checksum
        let mut p = TfLunaParser::default();
        for &b in frame.iter() {
            p.push(b);
        }
        assert_eq!(p.distance_cm(), None);
    }

    #[test]
    fn resync_after_garbage() {
        let mut p = TfLunaParser::default();
        // Feed garbage
        for &b in &[0x00u8, 0xFF, 0x12, 0xAB] {
            p.push(b);
        }
        // Then a valid frame
        for &b in make_frame(42).iter() {
            p.push(b);
        }
        assert_eq!(p.distance_cm(), Some(42));
    }

    #[test]
    fn stale_after_ticks() {
        use crate::config::STALE_TICKS;
        let mut p = TfLunaParser::default();
        for &b in make_frame(100).iter() {
            p.push(b);
        }
        assert_eq!(p.distance_cm(), Some(100));
        for _ in 0..STALE_TICKS {
            p.tick();
        }
        assert_eq!(p.distance_cm(), None);
    }
}
