mod display;

use std::fs::File;
use std::io::Read;
use std::sync::mpsc;
use std::thread;
use display::DisplaySize128x32Ssd1305;
use embedded_graphics::{
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use linux_embedded_hal::I2cdev;
use ssd1306::{prelude::*, I2CDisplayInterface, Ssd1306};

const NUM_BARS: usize = 16;
const BAR_WIDTH: u32 = 7;  // px per bar
const BAR_STEP: u32 = 8;   // bar_width + 1px gap; 16 bars × 8px = 128px
const DISPLAY_HEIGHT: u32 = 32;
const FRAME_BYTES: usize = NUM_BARS * 2; // 32 bytes, u16 LE per bar

fn main() {
    let i2c = I2cdev::new("/dev/i2c-1").expect("Failed to open /dev/i2c-1");
    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x32Ssd1305, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display.init().expect("Display init failed");

    // Reader thread drains cava.fifo continuously and hands each frame to the
    // render loop over an unbounded channel. The channel send never blocks, so
    // the slow (~15ms) I2C flush can't back up the FIFO read and stall the
    // CAVA → MPD pipeline (which would cause audio xruns). The render loop, in
    // turn, blocks on recv() until a fresh frame arrives — so we redraw at
    // CAVA's ~30fps cadence instead of free-running and flooding the I2C bus.
    let (tx, rx) = mpsc::channel::<[u8; FRAME_BYTES]>();
    thread::spawn(move || {
        let mut fifo = File::open("/tmp/cava.fifo").expect("Failed to open /tmp/cava.fifo");
        let mut buf = [0u8; FRAME_BYTES];
        loop {
            fifo.read_exact(&mut buf).expect("FIFO read error");
            if tx.send(buf).is_err() {
                break; // render side hung up; nothing left to do
            }
        }
    });

    let bar_style = PrimitiveStyle::with_fill(BinaryColor::On);
    let clear_style = PrimitiveStyle::with_fill(BinaryColor::Off);
    let full = Rectangle::new(Point::zero(), Size::new(128, 32));

    loop {
        // Block until the reader has a frame, then drain to the freshest one:
        // if the render flush ever falls behind, skip the stale frames and draw
        // only the newest so the display never lags behind the audio.
        let mut buf = match rx.recv() {
            Ok(f) => f,
            Err(_) => break, // reader thread gone — exit, let systemd restart us
        };
        while let Ok(f) = rx.try_recv() {
            buf = f;
        }

        // Clear framebuffer
        full.into_styled(clear_style).draw(&mut display).unwrap();

        // Draw each bar, bottom-up, with 1px gap on the right side
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
                .draw(&mut display)
                .unwrap();
        }

        display.flush().expect("Flush failed");
    }
}
