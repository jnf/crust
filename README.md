# crust

Real-time audio spectrum visualizer for Raspberry Pi Zero 2W + Adafruit OLED Bonnet.

## Hardware

| Component | Details |
|-----------|---------|
| Pi | Raspberry Pi Zero 2W, 64-bit Raspberry Pi OS Lite (aarch64) |
| Display | [Adafruit OLED Bonnet 4567](https://www.adafruit.com/product/4567) — 128×32px, SSD1305, I2C |
| I2C bus | `/dev/i2c-1`, address `0x3C`, 400kHz fast-mode |
| Audio out | micro-HDMI → HDMI audio extractor → S/PDIF → digital receiver |

## Signal path

```
MPD (playback)
  ├── ALSA → hw:vc4hdmi,0 → HDMI → extractor → S/PDIF → receiver
  └── FIFO output plugin → /tmp/mpd.fifo  (raw PCM, 44100/16bit/stereo)
                                  │
                               CAVA
                  (reads PCM FIFO, outputs spectrum bars at 30fps)
                                  │
                    /tmp/cava.fifo  (16 × u16 LE per frame)
                                  │
                               crust  (this binary)
                  (reads bar values, scales, renders rectangles)
                                  │
                            I2C @ 400kHz
                                  │
                         SSD1305 OLED (128×32)
```

## Pi-side dependencies

- `mpd` 0.24.4 — audio playback daemon
- `mpc` — MPD command-line client
- `cava` 0.10.4 — audio spectrum analyzer
- `i2c-tools` — for `i2cdetect` (diagnostics)

## Building

Cross-compile on macOS for aarch64 Linux using `cargo-zigbuild`:

```sh
# prerequisites (one-time)
rustup target add aarch64-unknown-linux-gnu
brew install zig
cargo install cargo-zigbuild

# build
cargo zigbuild --release

# deploy
scp target/aarch64-unknown-linux-gnu/release/crust jey@bars.local:~/
```

The default target is set in `.cargo/config.toml`, so plain `cargo zigbuild --release`
always builds for the Pi.

## Running (manual)

On the Pi, start the pipeline in order:

```sh
# 1. start MPD and queue music
sudo systemctl start mpd
mpc clear && mpc add / && mpc play

# 2. start CAVA (reads from MPD FIFO, writes spectrum to its own FIFO)
nohup cava -p ~/.config/cava/config > /tmp/cava.log 2>&1 &

# 3. start crust (reads CAVA FIFO, renders to OLED)
nohup ./crust > /tmp/crust.log 2>&1 &
```

`crust` blocks on the FIFO open until CAVA is running, so start CAVA first.

## Configuration files

### MPD — `/etc/mpd.conf` (relevant excerpts)

```
user            "jey"
music_directory "/home/jey/music"

audio_output {
    type        "alsa"
    name        "HDMI Audio"
    device      "plughw:vc4hdmi,0"
    mixer_type  "software"
}

audio_output {
    type        "fifo"
    name        "CAVA FIFO"
    path        "/tmp/mpd.fifo"
    format      "44100:16:2"
}
```

### CAVA — `~/.config/cava/config`

```ini
[general]
framerate = 30
bars = 16
bar_width = 1
bar_spacing = 0
sensitivity = 50

[input]
method = fifo
source = /tmp/mpd.fifo
sample_rate = 44100
sample_bits = 16
channels = 2

[output]
method = raw
raw_target = /tmp/cava.fifo
bit_format = 16bit
```

### I2C speed — `/boot/firmware/config.txt`

```
dtparam=i2c_arm=on
dtparam=i2c_arm_baudrate=400000
```

## Visualization — how the bars render

### CAVA output format

Each frame from `/tmp/cava.fifo` is exactly `NUM_BARS * 2` bytes: one
little-endian `u16` per bar, in frequency order (low → high). Values range
from `0` to `65535`.

```
frame = [ lo0, hi0, lo1, hi1, ..., lo15, hi15 ]   (32 bytes total)
```

### Bar layout on the 128×32 display

The SSD1305 has 132 column drivers for a 128px panel; hardware columns 0–3 are
off-screen on the left. `X_OFFSET=2` distributes the resulting 3px deficit across
both edge bars: bar 0 loses 2px (renders 5px, flush with left panel edge), bar 15
loses 1px (renders 6px). All 14 inner bars are 7px wide with 1px gaps.

```
const NUM_BARS: usize = 16;   // bars CAVA is configured to output
const BAR_WIDTH: u32  = 7;    // pixels wide per bar
const BAR_STEP: u32   = 8;    // pixels from one bar's left edge to the next
                               // = BAR_WIDTH (7) + 1px gap
const X_OFFSET: i32   = 2;    // splits SSD1305 3px deficit across edge bars
const DISPLAY_HEIGHT: u32 = 32; // pixel rows available
```

Layout (software x positions, bar index 0–15):

```
bar 0:  x = 2,   panel pixels 0–4    (5px visible — 2px off-screen left)
bar 1:  x = 10,  panel pixels 6–12   (7px)
bar 2:  x = 18,  panel pixels 14–20  (7px)
...
bar 15: x = 122, panel pixels 118–123 (6px — 1px clipped at software edge)
```

### Height scaling

CAVA outputs `u16` values (`0–65535`). These are scaled linearly to pixel
height (`0–32`):

```rust
let raw    = u16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]);
let height = (raw as u32 * DISPLAY_HEIGHT) / 65535;
```

Bars grow from the bottom of the display. The top-left corner of each bar
rectangle is computed as:

```rust
let x = (i as u32 * BAR_STEP) as i32;       // left edge of bar i
let y = (DISPLAY_HEIGHT - height) as i32;   // top edge (higher bar → smaller y)
```

A bar with `height = 0` is skipped entirely (nothing drawn). A bar with
`height = 32` fills the full column from `y=0` to `y=31`.

### Render loop

Each iteration:
1. `read_exact` — blocks until a full 32-byte frame arrives from the FIFO
2. Clear the framebuffer by drawing a black rectangle over the full display
3. For each bar: draw a white filled rectangle at `(x, y)` with size `(BAR_WIDTH, height)`
4. `flush()` — push the framebuffer to the display over I2C

The loop rate is naturally paced by CAVA's output (30fps target). No explicit
sleep or timer is needed.

### Checking system resource utilization

```sh
echo '=== CPU/MEM ===' && top -bn1 | head -20 && echo '=== per-process ===' && ps aux --sort=-%cpu | head -15
```

# TODOS
- Startup automation (systemd service)
- Catch sigkill (other exits?) and flush the OLED
