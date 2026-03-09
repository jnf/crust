use display_interface::{DisplayError, WriteOnlyDataCommand};
use ssd1306::{command::Command, prelude::DisplaySize};

/// SSD1305-specific display size for the Adafruit 128×32 OLED Bonnet.
///
/// The SSD1305 has 132 column drivers; hardware columns 0–3 are off the left edge of the panel.
/// OFFSETX = 4 maps software col 0 to hardware col 4 (first visible), so all 128 software pixels
/// land in the 128 visible hardware columns and no pixels are clipped.
#[derive(Debug, Copy, Clone)]
pub struct DisplaySize128x32Ssd1305;

impl DisplaySize for DisplaySize128x32Ssd1305 {
    const WIDTH: u8 = 128;
    const HEIGHT: u8 = 32;
    const DRIVER_COLS: u8 = 132;
    const OFFSETX: u8 = 4;
    type Buffer = [u8; 512]; // 128 * 32 / 8

    fn configure(&self, iface: &mut impl WriteOnlyDataCommand) -> Result<(), DisplayError> {
        // Same COM pin config as DisplaySize128x32: sequential, no remap
        Command::ComPinConfig(false, false).send(iface)
    }
}
