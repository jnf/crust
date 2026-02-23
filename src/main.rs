use std::fs::File;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use embedded_graphics::{
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use linux_embedded_hal::I2cdev;
use ssd1306::{prelude::*, I2CDisplayInterface, Ssd1306};

const NUM_BARS: usize = 16;
const BAR_WIDTH: u32 = 7;  // px per bar
const BAR_STEP: u32 = 8;   // bar_width + 1px gap
// SSD1305 has 132 col drivers; hw cols 0-3 are off-screen left, so a full
// X_OFFSET=4 clips bar 15 by 3px. X_OFFSET=2 splits the 3px deficit: bar 0
// loses 2px (shows 5px, flush with panel edge), bar 15 loses 1px (shows 6px).
const X_OFFSET: i32 = 2;
const DISPLAY_HEIGHT: u32 = 32;
const FRAME_BYTES: usize = NUM_BARS * 2; // 32 bytes, u16 LE per bar

fn main() {
    let i2c = I2cdev::new("/dev/i2c-1").expect("Failed to open /dev/i2c-1");
    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x32, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display.init().expect("Display init failed");

    // Reader thread drains cava.fifo continuously, decoupled from I2C writes.
    // Without this, the 15ms I2C flush blocks the FIFO read, backing up the
    // CAVA → MPD pipeline and causing audio xruns.
    let shared = Arc::new(Mutex::new([0u8; FRAME_BYTES]));
    let shared_reader = Arc::clone(&shared);
    thread::spawn(move || {
        let mut fifo = File::open("/tmp/cava.fifo").expect("Failed to open /tmp/cava.fifo");
        let mut buf = [0u8; FRAME_BYTES];
        loop {
            fifo.read_exact(&mut buf).expect("FIFO read error");
            *shared_reader.lock().unwrap() = buf;
        }
    });

    let bar_style = PrimitiveStyle::with_fill(BinaryColor::On);
    let clear_style = PrimitiveStyle::with_fill(BinaryColor::Off);
    let full = Rectangle::new(Point::zero(), Size::new(128, 32));

    loop {
        let buf = *shared.lock().unwrap();

        // Clear framebuffer
        full.into_styled(clear_style).draw(&mut display).unwrap();

        // Draw each bar, bottom-up, with 1px gap on the right side
        for i in 0..NUM_BARS {
            let raw = u16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]);
            let height = (raw as u32 * DISPLAY_HEIGHT) / 65535;
            if height == 0 {
                continue;
            }
            let x = X_OFFSET + (i as u32 * BAR_STEP) as i32;
            let y = (DISPLAY_HEIGHT - height) as i32;
            Rectangle::new(Point::new(x, y), Size::new(BAR_WIDTH, height))
                .into_styled(bar_style)
                .draw(&mut display)
                .unwrap();
        }

        display.flush().expect("Flush failed");
    }
}
