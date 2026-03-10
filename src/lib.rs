use embedded_graphics::{
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};

pub const NUM_BARS: usize = 16;
pub const BAR_WIDTH: u32 = 7;
pub const BAR_STEP: u32 = 8;
pub const DISPLAY_HEIGHT: u32 = 32;
pub const FRAME_BYTES: usize = NUM_BARS * 2;

/// Render one spectrum frame onto any embedded-graphics DrawTarget.
/// Clears the display, then draws filled white rectangles for each bar.
/// `buf` is a 32-byte CAVA frame: 16 × u16 LE bar values (0–65535).
pub fn render_frame<D>(display: &mut D, buf: &[u8; FRAME_BYTES]) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    let clear_style = PrimitiveStyle::with_fill(BinaryColor::Off);
    let bar_style = PrimitiveStyle::with_fill(BinaryColor::On);

    Rectangle::new(Point::zero(), Size::new(128, 32))
        .into_styled(clear_style)
        .draw(display)?;

    for i in 0..NUM_BARS {
        let raw = u16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]);
        let height = (raw as u32 * DISPLAY_HEIGHT) / 65535;
        if height == 0 {
            continue;
        }
        let x = (i as u32 * BAR_STEP) as i32;
        let y = (DISPLAY_HEIGHT - height) as i32;
        Rectangle::new(Point::new(x, y), Size::new(BAR_WIDTH, height))
            .into_styled(bar_style)
            .draw(display)?;
    }
    Ok(())
}
