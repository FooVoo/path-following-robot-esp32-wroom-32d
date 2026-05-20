//! Display output port — write text to a character display.
//!
//! Implemented by [`crate::adapters::esp32::lcd1602::Lcd1602`] for the 1602
//! LCD in 4-bit parallel mode.  [`crate::domain::robot::NoDisplay`] provides
//! a silent no-op default so all existing code compiles without a display.

/// Character-display output port.
pub trait DisplayPort {
    /// Write `text` to `row` (0-indexed).
    ///
    /// Text longer than the physical display width is silently truncated.
    /// Text shorter than the display width should be padded with spaces so
    /// that stale characters from the previous write are erased.
    fn print_row(&mut self, row: u8, text: &str);

    /// Clear the entire display.
    fn clear(&mut self);
}
