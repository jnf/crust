mod display;

use std::fs::File;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use crust::{render_frame, FRAME_BYTES};
use display::DisplaySize128x32Ssd1305;
use linux_embedded_hal::I2cdev;
use ssd1306::{prelude::*, I2CDisplayInterface, Ssd1306};

fn main() {
    let i2c = I2cdev::new("/dev/i2c-1").expect("Failed to open /dev/i2c-1");
    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x32Ssd1305, DisplayRotation::Rotate0)
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

    loop {
        let buf = *shared.lock().unwrap();
        render_frame(&mut display, &buf).unwrap();
        display.flush().expect("Flush failed");
    }
}
