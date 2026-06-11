# crust

Real-time audio spectrum visualizer for Raspberry Pi Zero 2W + Adafruit OLED Bonnet.

## Hardware

| Component | Details |
|-----------|---------|
| Pi | Raspberry Pi Zero 2W, 64-bit Raspberry Pi OS Lite (aarch64) |
| Display | [Adafruit OLED Bonnet 4567](https://www.adafruit.com/product/4567) — 128×32px, SSD1305, I2C |
| I2C bus | `/dev/i2c-1`, address `0x3C`, 400kHz fast-mode |
| Audio signal chain | USB → NAD D3045 (USB DAC), behind a USB hub. Fallback: micro-HDMI → HDMI audio extractor → S/PDIF → receiver |

## Signal path

```
MPD (playback)
  ├── ALSA → plughw:CARD=Audio,0 → USB → NAD D3045 DAC      (primary)
  │   └─ fallback: plughw:vc4hdmi,0 → HDMI → extractor → S/PDIF → receiver
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

- `mpd` 0.24.4: audio playback daemon
- `mpc`: MPD command-line client
- `cava` 0.10.4: audio spectrum analyzer
- `i2c-tools`: for `i2cdetect` (diagnostics)

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
scp target/aarch64-unknown-linux-gnu/release/crust <user>@<pi>:~/
```

The default target is set in `.cargo/config.toml`, so plain `cargo zigbuild --release` builds for the Pi.

See [SETUP.md](SETUP.md) for first-time Pi setup: dependencies, config files, systemd services, and deployment steps.

## MPD shell aliases

`scripts/mpc-aliases.sh` wraps common `mpc` commands with short names. Source it from `~/.bashrc` on the Pi:

```sh
source ~/crust/scripts/mpc-aliases.sh
```

### Playback

```sh
ms          # status: current track, play/pause state, progress, volume
mc          # current track name only
mp          # play/pause toggle
mn          # next track
mb          # back — cdprev: restarts current track if past ~3s, else goes to previous
mx          # stop
```

### Volume

```sh
mvo 80      # set volume to 80%
mup         # volume +5%
mdn         # volume -5%
```

### Queue

```sh
mq          # show current queue
mclear      # clear the queue
mshuffle    # shuffle the queue in place
```

### Library

`mfind` and `madd` both accept an optional tag as the first argument (`artist`, `album`, `title`, etc.). Without a tag they search all fields.

```sh
mfind "ok computer"            # search all fields, print matches
mfind artist "radiohead"       # search by artist tag

madd "ok computer"             # add all matches to current queue
madd album "ok computer"       # add by album tag

mplay "in rainbows"            # clear queue, add matches, start playing
mplay artist "radiohead"       # same, filtered by artist tag

mls                            # list library root
mls "Radiohead"                # list contents of a directory
```

### Now-playing notifications

`mwatch` uses `mpc idleloop` to listen for MPD player events and react without polling.

```sh
mwatch                         # print track name to terminal on each change
mwatch title &                 # update terminal title bar in the background
```

The background variant is useful in an active SSH session: run it once and your terminal's title bar tracks the current song for the rest of the session. It exits automatically when the SSH session ends. To stop it early: `kill %1` (or check `jobs` for the job number).

## Visualization — how the bars render

### CAVA output format

Each frame from `/tmp/cava.fifo` is exactly `NUM_BARS * 2` bytes: one little-endian `u16` per bar, in frequency order (low → high). Values range from `0` to `65535`.

```
frame = [ lo0, hi0, lo1, hi1, ..., lo15, hi15 ]   (32 bytes total)
```

### Bar layout on the 128×32 display

The SSD1305 has 132 column drivers for a 128px panel; hardware columns 0–3 are off-screen on the left. The custom `DisplaySize128x32Ssd1305` type in `src/display.rs` sets `DRIVER_COLS=132` and `OFFSETX=4`, which tells the ssd1306 driver to start column addressing at hardware column 4. Software column 0 maps to the first visible pixel, software column 127 maps to the last, and all 16 bars fit exactly with no clipping.

```
const NUM_BARS: usize = 16;   // bars CAVA is configured to output
const BAR_WIDTH: u32  = 7;    // pixels wide per bar
const BAR_STEP: u32   = 8;    // pixels from one bar's left edge to the next
                               // = BAR_WIDTH (7) + 1px gap; 16 × 8 = 128px
const DISPLAY_HEIGHT: u32 = 32; // pixel rows available
```

Layout (software x positions, bar index 0–15):

```
bar 0:  x = 0,   pixels 0–6    (7px, flush with left edge)
bar 1:  x = 8,   pixels 8–14   (7px)
bar 2:  x = 16,  pixels 16–22  (7px)
...
bar 15: x = 120, pixels 120–126 (7px, flush with right edge)
```

### Height scaling

CAVA outputs `u16` values (`0–65535`). These are scaled linearly to pixel height (`0–32`):

```rust
let raw    = u16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]);
let height = (raw as u32 * DISPLAY_HEIGHT) / 65535;
```

Bars grow from the bottom of the display. The top-left corner of each bar rectangle is computed as:

```rust
let x = (i as u32 * BAR_STEP) as i32;       // left edge of bar i
let y = (DISPLAY_HEIGHT - height) as i32;   // top edge (higher bar → smaller y)
```

A bar with `height = 0` is skipped entirely (nothing drawn). A bar with `height = 32` fills the full column from `y=0` to `y=31`.

### Render loop

Each iteration:
1. `read_exact` — blocks until a full 32-byte frame arrives from the FIFO
2. Clear the framebuffer by drawing a black rectangle over the full display
3. For each bar: draw a white filled rectangle at `(x, y)` with size `(BAR_WIDTH, height)`
4. `flush()` — push the framebuffer to the display over I2C

The loop rate is naturally paced by CAVA's output (30fps target). No explicit sleep or timer is needed.

