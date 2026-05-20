//! SSD1306 OLED display adapter — 128×64, I²C (4-pin), address 0x3C.
//!
//! # Connections
//!
//! | Pin | Function  | Notes                                                            |
//! |-----|-----------|------------------------------------------------------------------|
//! | VCC | Power     | 3.3 V only — do **not** connect to 5 V                          |
//! | GND | Ground    |                                                                  |
//! | SCL | I²C clock | Shared with TCA9548A / VL53L0X; external 4.7 kΩ pull-up needed |
//! | SDA | I²C data  | Shared with TCA9548A / VL53L0X; external 4.7 kΩ pull-up needed |
//!
//! The SA0 address bit is pulled low on most common modules → address **0x3C**.
//! If your module has SA0 tied to VCC, change [`SSD1306_I2C_ADDR`] in
//! `src/config.rs` to `0x3D`.
//!
//! # Display layout
//!
//! The display uses [`FONT_6X10`], giving ≈21 characters per row.
//! Only two rows are exposed via [`DisplayPort`]:
//!
//! | Row | Y offset | Typical content   |
//! |-----|----------|-------------------|
//! |  0  |    4 px  | FSM state         |
//! |  1  |   36 px  | LIDAR distance    |
//!
//! Each `print_row` call erases a 128×22 px horizontal band before writing,
//! so stale text from the previous write is always overwritten.
//!
//! # Sharing the I²C bus
//!
//! The driver takes a `&RefCell<I2C>` rather than exclusive ownership of the
//! bus so that the same `I2c` peripheral can be shared with other I²C devices
//! (TCA9548A mux, VL53L0X sensors).  A lightweight [`I2cBorrow`] newtype
//! handles the borrow-checked interior-mutability bridging.

use core::cell::RefCell;

use display_interface_i2c::I2CInterface;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyleBuilder},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Baseline, Text},
};
use log::trace;
use ssd1306::{
    I2CDisplayInterface, Ssd1306,
    mode::BufferedGraphicsMode,
    prelude::DisplayRotation,
    size::DisplaySize128x64,
};

use crate::ports::display::DisplayPort;

// ── Layout constants ──────────────────────────────────────────────────────────

/// Y-coordinate (top of erase band / text baseline) for row 0.
const ROW0_Y: i32 = 4;
/// Y-coordinate (top of erase band / text baseline) for row 1.
const ROW1_Y: i32 = 36;
/// Height of the erase band for each row — font height (10 px) + 2 px margin.
const ROW_BAND_H: u32 = 22;

// ── I²C borrow shim ───────────────────────────────────────────────────────────

/// Newtype around `&RefCell<T>` that implements `embedded_hal::i2c::I2c`.
///
/// `ssd1306` requires exclusive ownership of the I²C device.  This shim
/// borrows the `RefCell` for each transaction using `borrow_mut()`, which
/// is safe in a single-threaded embedded context where no two drivers call
/// into the bus at the same time.
struct I2cBorrow<'a, T>(pub &'a RefCell<T>);

impl<'a, T: embedded_hal::i2c::ErrorType> embedded_hal::i2c::ErrorType for I2cBorrow<'a, T> {
    type Error = T::Error;
}

impl<'a, T: embedded_hal::i2c::I2c> embedded_hal::i2c::I2c for I2cBorrow<'a, T> {
    fn transaction(
        &mut self,
        address: u8,
        operations: &mut [embedded_hal::i2c::Operation<'_>],
    ) -> Result<(), Self::Error> {
        self.0.borrow_mut().transaction(address, operations)
    }
}

// ── Adapter ───────────────────────────────────────────────────────────────────

type Driver<'a, I2C> = Ssd1306<
    I2CInterface<I2cBorrow<'a, I2C>>,
    DisplaySize128x64,
    BufferedGraphicsMode<DisplaySize128x64>,
>;

/// SSD1306 128×64 OLED display driven over a shared I²C bus.
///
/// Created with [`Ssd1306Display::init`]; subsequently used through the
/// [`DisplayPort`] trait for state and sensor readouts.
pub struct Ssd1306Display<'a, I2C> {
    driver: Driver<'a, I2C>,
}

impl<'a, I2C: embedded_hal::i2c::I2c> Ssd1306Display<'a, I2C> {
    /// Initialise the SSD1306 and clear the screen.
    ///
    /// Borrows the shared `RefCell<I2C>` for the lifetime of this adapter.
    ///
    /// # Panics
    ///
    /// Panics if the SSD1306 does not acknowledge its address on the bus
    /// (bad wiring, wrong address, missing pull-ups).
    pub fn init(i2c_cell: &'a RefCell<I2C>) -> Self {
        let iface = I2CDisplayInterface::new_custom_address(
            I2cBorrow(i2c_cell),
            crate::config::SSD1306_I2C_ADDR,
        );
        let mut driver = Ssd1306::new(iface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        driver.init().expect("SSD1306 init failed");
        driver.clear(BinaryColor::Off).ok();
        driver.flush().ok();
        Self { driver }
    }
}

impl<'a, I2C: embedded_hal::i2c::I2c> DisplayPort for Ssd1306Display<'a, I2C> {
    /// Write `text` to the given display row (0 or 1), erasing stale content.
    ///
    /// Non-displayable characters are passed through as-is; `embedded-graphics`
    /// substitutes a placeholder glyph for any code point missing from the font.
    /// Strings longer than ≈21 characters are silently clipped at the right edge.
    fn print_row(&mut self, row: u8, text: &str) {
        let y = match row {
            0 => ROW0_Y,
            1 => ROW1_Y,
            _ => return,
        };
        trace!("OLED row {}: \"{}\"", row, text);

        // Erase the row's pixel band to remove stale characters.
        let erase_style = PrimitiveStyleBuilder::new()
            .fill_color(BinaryColor::Off)
            .build();
        Rectangle::new(Point::new(0, y - 2), Size::new(128, ROW_BAND_H))
            .into_styled(erase_style)
            .draw(&mut self.driver)
            .ok();

        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();
        Text::with_baseline(text, Point::new(0, y), text_style, Baseline::Top)
            .draw(&mut self.driver)
            .ok();

        self.driver.flush().ok();
    }

    fn clear(&mut self) {
        self.driver.clear(BinaryColor::Off).ok();
        self.driver.flush().ok();
    }
}
