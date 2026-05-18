//! ESP32 joystick + button adapter.
//!
//! Two ADC channels (X-axis, Y-axis) are sampled and mixed into signed
//! throttle values for left / right motors.  A GPIO input is debounced for
//! the button.
//!
//! # Axis mixing
//!
//! ```text
//! y-axis (forward/back) ──►  mix  ──► throttle_left
//! x-axis (left/right)   ──►       ──► throttle_right
//! ```
//!
//! With a dead-zone of ±DEAD_ZONE_RAW around mid-scale, and the following
//! formula:
//!
//! ```
//! throttle_l =  y + x   (clamped to ±100)
//! throttle_r =  y - x   (clamped to ±100)
//! ```
//!
//! # Button debounce
//!
//! A rising-edge is reported at most once per `DEBOUNCE_MS` window.

use log::{debug, trace};

use esp_hal::{
    analog::adc::{Adc, AdcChannel, AdcPin, RegisterAccess},
    gpio::Input,
    Blocking,
};
use nb;

use crate::{
    config::{ADC_MAX_RETRIES, DEAD_ZONE_RAW, DEBOUNCE_MS, JOY_CENTER_RAW},
    ports::input::InputPort,
};

// ---------------------------------------------------------------------------
// Axis mapping helpers (no HAL dependency — testable on host)
// ---------------------------------------------------------------------------

/// Convert a raw ADC reading [0, 4095] to a signed throttle value [-100, 100].
///
/// The mapping is:
///  * `raw ≤ center - dead` → -100 … 0  (linear)
///  * `|raw - center| < dead` → 0        (dead zone)
///  * `raw ≥ center + dead` → 0 … +100  (linear)
pub fn map_axis(raw: u16) -> i8 {
    let raw = raw as i32;
    let center = JOY_CENTER_RAW as i32;
    let dead = DEAD_ZONE_RAW as i32;

    let delta = raw - center;
    if delta.abs() < dead {
        return 0;
    }

    let max_range = center.max(4095 - center); // distance to rail
    let val = ((delta.abs() - dead) * 100) / (max_range - dead).max(1);
    let val = val.clamp(0, 100);

    if delta < 0 { -(val as i8) } else { val as i8 }
}

/// Mix joystick axes into differential-drive throttles.
///
/// Returns `(throttle_left, throttle_right)`, each in `[-100, 100]`.
pub fn mix_drive(x: i8, y: i8) -> (i8, i8) {
    let l = (y as i16 + x as i16).clamp(-100, 100) as i8;
    let r = (y as i16 - x as i16).clamp(-100, 100) as i8;
    (l, r)
}

// ---------------------------------------------------------------------------
// Adapter struct
// ---------------------------------------------------------------------------

/// ESP32 joystick adapter.
///
/// Generic over:
/// * `'d`   — peripheral lifetime
/// * `ADCI` — ADC interface type (implements [`RegisterAccess`])
/// * `PX`   — GPIO pin type for the X axis (implements [`AdcChannel`])
/// * `PY`   — GPIO pin type for the Y axis (implements [`AdcChannel`])
pub struct Esp32Joystick<'d, ADCI, PX, PY> {
    adc: Adc<'d, ADCI, Blocking>,
    pin_x: AdcPin<PX, ADCI>,
    pin_y: AdcPin<PY, ADCI>,
    btn: Input<'d>,
    /// `now_ms` of the last confirmed button press (for debounce).
    last_btn_ms: u64,
    /// Level seen during the last debounce window.
    last_btn_level: bool,
    /// One-shot flag: set to `true` when a press edge is confirmed.
    btn_event: bool,
    throttle_l: i8,
    throttle_r: i8,
}

impl<'d, ADCI, PX, PY> Esp32Joystick<'d, ADCI, PX, PY>
where
    ADCI: RegisterAccess + 'd,
    PX: AdcChannel,
    PY: AdcChannel,
{
    /// Construct the adapter.
    pub fn new(
        adc: Adc<'d, ADCI, Blocking>,
        pin_x: AdcPin<PX, ADCI>,
        pin_y: AdcPin<PY, ADCI>,
        btn: Input<'d>,
    ) -> Self {
        Self {
            adc,
            pin_x,
            pin_y,
            btn,
            last_btn_ms: 0,
            last_btn_level: false,
            btn_event: false,
            throttle_l: 0,
            throttle_r: 0,
        }
    }

    /// Read one ADC axis with a retry loop.
    ///
    /// Falls back to `JOY_CENTER_RAW` after `ADC_MAX_RETRIES` attempts so
    /// the main loop is never blocked indefinitely.
    fn read_axis(adc: &mut Adc<'d, ADCI, Blocking>, pin: &mut AdcPin<PX, ADCI>) -> u16
    where
        PX: AdcChannel,
    {
        for _ in 0..ADC_MAX_RETRIES {
            match adc.read_oneshot(pin) {
                Ok(v) => return v,
                Err(nb::Error::WouldBlock) => continue,
                Err(_) => break,
            }
        }
        debug!("ADC X axis read timeout — using center");
        JOY_CENTER_RAW
    }

    /// Read one ADC axis with a retry loop (Y pin variant).
    fn read_axis_y(adc: &mut Adc<'d, ADCI, Blocking>, pin: &mut AdcPin<PY, ADCI>) -> u16
    where
        PY: AdcChannel,
    {
        for _ in 0..ADC_MAX_RETRIES {
            match adc.read_oneshot(pin) {
                Ok(v) => return v,
                Err(nb::Error::WouldBlock) => continue,
                Err(_) => break,
            }
        }
        debug!("ADC Y axis read timeout — using center");
        JOY_CENTER_RAW
    }
}

impl<'d, ADCI, PX, PY> InputPort for Esp32Joystick<'d, ADCI, PX, PY>
where
    ADCI: RegisterAccess + 'd,
    PX: AdcChannel,
    PY: AdcChannel,
{
    fn poll(&mut self, now_ms: u64) {
        // --- ADC sampling ---
        let raw_x = Self::read_axis(&mut self.adc, &mut self.pin_x);
        let raw_y = Self::read_axis_y(&mut self.adc, &mut self.pin_y);
        trace!("joystick raw x={} y={}", raw_x, raw_y);

        let ax = map_axis(raw_x);
        let ay = map_axis(raw_y);
        let (tl, tr) = mix_drive(ax, ay);
        self.throttle_l = tl;
        self.throttle_r = tr;

        // --- Button debounce ---
        let pressed = self.btn.is_low();
        if pressed && !self.last_btn_level {
            // Rising edge — check debounce window.
            if now_ms.saturating_sub(self.last_btn_ms) >= DEBOUNCE_MS {
                debug!("button press confirmed at {}ms", now_ms);
                self.btn_event = true;
                self.last_btn_ms = now_ms;
            }
        }
        self.last_btn_level = pressed;
    }

    fn throttle_left(&self) -> i8 {
        self.throttle_l
    }

    fn throttle_right(&self) -> i8 {
        self.throttle_r
    }

    fn take_button_press(&mut self) -> bool {
        let v = self.btn_event;
        self.btn_event = false;
        v
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{map_axis, mix_drive};

    #[test]
    fn center_is_zero() {
        use crate::config::JOY_CENTER_RAW;
        assert_eq!(map_axis(JOY_CENTER_RAW), 0);
    }

    #[test]
    fn full_forward() {
        assert_eq!(map_axis(4095), 100);
    }

    #[test]
    fn full_reverse() {
        assert_eq!(map_axis(0), -100);
    }

    #[test]
    fn dead_zone_low_edge() {
        use crate::config::{DEAD_ZONE_RAW, JOY_CENTER_RAW};
        assert_eq!(map_axis(JOY_CENTER_RAW - DEAD_ZONE_RAW + 1), 0);
    }

    #[test]
    fn mix_straight_forward() {
        let (l, r) = mix_drive(0, 100);
        assert_eq!(l, 100);
        assert_eq!(r, 100);
    }

    #[test]
    fn mix_turn_right() {
        let (l, r) = mix_drive(50, 0);
        assert_eq!(l, 50);
        assert_eq!(r, -50);
    }

    #[test]
    fn mix_clamps_to_100() {
        let (l, _r) = mix_drive(100, 100);
        assert_eq!(l, 100);
    }
}
