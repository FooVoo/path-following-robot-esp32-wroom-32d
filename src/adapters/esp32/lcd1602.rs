//! 1602 LCD adapter — HD44780 controller, 4-bit parallel interface.
//!
//! # Connections (write-only — tie the RW pin to GND)
//!
//! | LCD pin | Function        | ESP32 GPIO constant |
//! |---------|-----------------|---------------------|
//! | RS      | Register select | `config::LCD_RS_GPIO` |
//! | EN      | Enable clock    | `config::LCD_EN_GPIO` |
//! | D4      | Data bit 4      | `config::LCD_D4_GPIO` |
//! | D5      | Data bit 5      | `config::LCD_D5_GPIO` |
//! | D6      | Data bit 6      | `config::LCD_D6_GPIO` |
//! | D7      | Data bit 7      | `config::LCD_D7_GPIO` |
//!
//! RW is not connected to firmware; tie it to GND for permanent write mode.
//! Contrast (V0) is set via a 10 kΩ potentiometer or fixed resistor to GND.
//! Backlight anode (A) and cathode (K) are optional; connect via 100 Ω
//! current-limiting resistor to 3.3 V and GND respectively.
//!
//! # Timing
//!
//! All delays come from the HD44780 datasheet.  The longest mandatory delay
//! is 15 ms at power-on, which is issued at the start of [`Lcd1602::new`].

use log::trace;

use esp_hal::{
    delay::Delay,
    gpio::{Level, Output},
};

use crate::ports::display::DisplayPort;

// ── Display geometry ──────────────────────────────────────────────────────────

const LCD_COLS: usize = 16;
const LCD_ROWS: usize = 2;

/// DDRAM start address for each row on a standard 16×2 display.
const ROW_DDRAM: [u8; 2] = [0x00, 0x40];

// ── HD44780 timing (microseconds) ─────────────────────────────────────────────

const T_POWER_ON_US:  u32 = 15_000; // ≥15 ms after VCC rise
const T_INIT_1_US:    u32 =  4_100; // ≥4.1 ms (first function-set retry)
const T_INIT_2_US:    u32 =    150; // ≥100 µs (second retry)
const T_CMD_US:       u32 =     50; // ≥37 µs for most commands
const T_CLEAR_US:     u32 =  2_000; // ≥1.52 ms for Clear/Return-Home
const T_ENABLE_US:    u32 =      1; // ≥1 µs EN pulse width

// ── Adapter ───────────────────────────────────────────────────────────────────

/// HD44780-compatible 16×2 LCD driven in 4-bit mode.
pub struct Lcd1602<'d> {
    rs:    Output<'d>,
    en:    Output<'d>,
    d4:    Output<'d>,
    d5:    Output<'d>,
    d6:    Output<'d>,
    d7:    Output<'d>,
    delay: Delay,
}

impl<'d> Lcd1602<'d> {
    /// Initialise the display.
    ///
    /// Runs the full HD44780 4-bit initialisation sequence including the
    /// mandatory power-on delay.  The display is blank and ready after this
    /// returns.
    pub fn new(
        rs: Output<'d>,
        en: Output<'d>,
        d4: Output<'d>,
        d5: Output<'d>,
        d6: Output<'d>,
        d7: Output<'d>,
    ) -> Self {
        let mut lcd = Self { rs, en, d4, d5, d6, d7, delay: Delay::new() };
        lcd.do_init();
        lcd
    }

    // ── Initialisation ────────────────────────────────────────────────────────

    fn do_init(&mut self) {
        // Wait for VCC to stabilise (HD44780 table 12, note 5).
        self.delay.delay_micros(T_POWER_ON_US);

        // Three 8-bit mode probes — required soft-reset before 4-bit switch.
        self.write_nibble(0x03);
        self.delay.delay_micros(T_INIT_1_US);
        self.write_nibble(0x03);
        self.delay.delay_micros(T_INIT_2_US);
        self.write_nibble(0x03);
        self.delay.delay_micros(T_CMD_US);

        // Switch to 4-bit mode.
        self.write_nibble(0x02);
        self.delay.delay_micros(T_CMD_US);

        // Function set: DL=0 (4-bit), N=1 (2 lines), F=0 (5×8 dots).
        self.send_cmd(0x28);
        // Display off.
        self.send_cmd(0x08);
        // Clear display (needs extra delay).
        self.send_cmd(0x01);
        self.delay.delay_micros(T_CLEAR_US);
        // Entry mode: cursor increments, no display shift.
        self.send_cmd(0x06);
        // Display on; cursor and blink both off.
        self.send_cmd(0x0C);
    }

    // ── Low-level I/O ─────────────────────────────────────────────────────────

    /// Output 4 bits on D4–D7 then pulse EN high → low.
    fn write_nibble(&mut self, nibble: u8) {
        self.d4.set_level(if nibble & 0x01 != 0 { Level::High } else { Level::Low });
        self.d5.set_level(if nibble & 0x02 != 0 { Level::High } else { Level::Low });
        self.d6.set_level(if nibble & 0x04 != 0 { Level::High } else { Level::Low });
        self.d7.set_level(if nibble & 0x08 != 0 { Level::High } else { Level::Low });
        self.en.set_high();
        self.delay.delay_micros(T_ENABLE_US);
        self.en.set_low();
        self.delay.delay_micros(T_CMD_US);
    }

    /// Send a full byte as two 4-bit nibbles (high nibble first).
    ///
    /// `rs = false` → command register; `rs = true` → data register.
    fn send_byte(&mut self, rs: bool, byte: u8) {
        self.rs.set_level(if rs { Level::High } else { Level::Low });
        self.write_nibble(byte >> 4);
        self.write_nibble(byte & 0x0F);
    }

    fn send_cmd(&mut self, cmd: u8) {
        self.send_byte(false, cmd);
    }

    fn send_data(&mut self, data: u8) {
        self.send_byte(true, data);
    }

    /// Move the cursor to column `col`, row `row` (both 0-indexed).
    fn set_cursor(&mut self, col: u8, row: u8) {
        let row = row as usize % LCD_ROWS;
        self.send_cmd(0x80 | (ROW_DDRAM[row] + col));
    }
}

impl<'d> DisplayPort for Lcd1602<'d> {
    fn print_row(&mut self, row: u8, text: &str) {
        if row as usize >= LCD_ROWS {
            return;
        }
        trace!("LCD row {}: \"{}\"", row, text);
        self.set_cursor(0, row);

        let mut col = 0usize;
        for ch in text.chars() {
            if col >= LCD_COLS {
                break;
            }
            // Map non-ASCII to '?' — HD44780 ROM-A00 only covers ASCII.
            self.send_data(if ch.is_ascii() { ch as u8 } else { b'?' });
            col += 1;
        }
        // Pad with spaces to overwrite any stale characters from the previous write.
        while col < LCD_COLS {
            self.send_data(b' ');
            col += 1;
        }
    }

    fn clear(&mut self) {
        self.send_cmd(0x01);
        self.delay.delay_micros(T_CLEAR_US);
    }
}
