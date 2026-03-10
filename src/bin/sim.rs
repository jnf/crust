use std::f32::consts::TAU;
use std::thread;
use std::time::Duration;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::Size;
use embedded_graphics_simulator::{
    BinaryColorTheme, OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent, Window,
};
use crust::{render_frame, FRAME_BYTES, NUM_BARS};

fn main() {
    let output_settings = OutputSettingsBuilder::new()
        .theme(BinaryColorTheme::OledWhite)
        .scale(2)
        .build();
    let mut window = Window::new("crust simulator", &output_settings);
    let mut display = SimulatorDisplay::<BinaryColor>::new(Size::new(128, 32));

    let mut t: f32 = 0.0;
    'running: loop {
        // Synthetic: sine sweep — each bar is a different phase of a slow oscillation
        let mut buf = [0u8; FRAME_BYTES];
        for i in 0..NUM_BARS {
            let phase = (i as f32 / NUM_BARS as f32) * TAU;
            let val = (((t + phase).sin() * 0.5 + 0.5) * 65535.0) as u16;
            let [lo, hi] = val.to_le_bytes();
            buf[i * 2] = lo;
            buf[i * 2 + 1] = hi;
        }
        t += 0.05;

        render_frame(&mut display, &buf).unwrap();
        window.update(&display);

        for event in window.events() {
            if let SimulatorEvent::Quit = event {
                break 'running;
            }
        }

        thread::sleep(Duration::from_millis(33)); // ~30fps
    }
}
