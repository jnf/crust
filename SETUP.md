# crust — setup guide

Step-by-step instructions for a competent tinkerer setting up crust on a fresh (but already-booted) Raspberry Pi Zero 2W.

**Assumed:** SSH access works, Raspberry Pi OS Lite (64-bit, aarch64) is running.

---

## 1. Hardware you'll need

| Item | Notes |
|------|-------|
| Raspberry Pi Zero 2W | 64-bit OS required |
| [Adafruit OLED Bonnet 4567](https://www.adafruit.com/product/4567) | 128×32px, SSD1305, I2C — seat it on the GPIO header |
| Audio signal chain | crust drives a visualizer; it needs audio input. This build uses micro-HDMI → HDMI audio extractor → S/PDIF coax → receiver. Any setup that routes audio through MPD on the Pi will work, but **the Pi's HDMI port must have an active downstream display or extractor asserting HPD** — without it the vc4-hdmi audio driver never initialises and MPD fails to open the device. |

---

## 2. Install dependencies

```sh
sudo apt update
sudo apt install -y mpd mpc cava i2c-tools
```

---

## 3. Enable I2C at 400 kHz

Add or update `/boot/firmware/config.txt`:

```
dtparam=i2c_arm=on
dtparam=i2c_arm_baudrate=400000
```

Reboot, then verify the OLED is visible:

```sh
i2cdetect -y 1   # should show 0x3c
```

---

## 4. Configure MPD

Edit `/etc/mpd.conf`. The critical settings:

```
music_directory    "/home/<user>/music"
audio_buffer_size  "32768"    # 32 MB — prevents xruns caused by SD card latency

audio_output {
    type        "alsa"
    name        "HDMI Audio"
    device      "plughw:vc4hdmi,0"   # plughw, not hw — required for PCM→IEC958 conversion
    mixer_type  "software"
}

audio_output {
    type    "fifo"
    name    "CAVA FIFO"
    path    "/tmp/mpd.fifo"
    format  "44100:16:2"
}
```

> **Note:** `plughw:vc4hdmi,0` is required. The bare `hw:` device only accepts IEC958 subframe format; `plughw` enables the ALSA plugin layer that converts standard PCM transparently.

---

## 5. Configure CAVA

Create `~/.config/cava/config`:

```ini
[general]
framerate = 30
bars = 16
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

> **Important:** keep `bars = 16`. CAVA seems to always writes 32 bytes per frame regardless of the bars setting; using other values for bars causes frame-boundary drift.

---

## 6. Set up the MPD HDMI wait drop-in

The vc4-hdmi audio driver requires a live HDMI link before it can be opened. On a cold boot this negotiation can take a few minutes. Without this drop-in, MPD starts too early and silently fails to open its audio output (error 524).

```sh
sudo mkdir -p /etc/systemd/system/mpd.service.d
sudo cp systemd/mpd.service.d/10-hdmi-wait.conf /etc/systemd/system/mpd.service.d/
```

The drop-in loops `aplay` against the device every 5 seconds until it succeeds, then unblocks MPD. Timeout is 10 minutes.

---

## 7. Install systemd services

```sh
sudo cp systemd/cava.service  /etc/systemd/system/
sudo cp systemd/crust.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable mpd cava crust
```

All three services will start automatically on boot in the correct order: MPD (after HDMI link) → CAVA → crust.

---

## 8. Build and deploy crust

On your **development machine** (assuming macOS with Rust installed):

One-time setup:

```sh
rustup target add aarch64-unknown-linux-gnu
brew install zig
cargo install cargo-zigbuild
```

Build on dev machine, push to Pi on the network:

```
cargo zigbuild --release
scp target/aarch64-unknown-linux-gnu/release/crust <user>@<pi>:~/
```

---

## 9. First run

Add music to MPD's library (put your audio files in `~/music`) and start everything:

```sh
# On the Pi
mpc update && mpc add / && mpc play
sudo systemctl start cava crust
```

The OLED should show a live spectrum within a second or two.

---

## 10. Utilities

**Clear the display** (stop crust and blank the OLED):

```sh
~/clear-oled.sh
```

**Restart the visualizer:**

```sh
sudo systemctl start crust
mpc play
```

**Check service health:**

```sh
systemctl status mpd cava crust
```

**Check for audio xruns** (should be empty during normal playback):

```sh
journalctl -u mpd --since "10 minutes ago" | grep xrun
```

**Check CPU/memory usage:**

```sh
echo '=== CPU/MEM ===' && top -bn1 | head -20 && echo '=== per-process ===' && ps aux --sort=-%cpu | head -15
```

---

For technical details on the visualization pipeline and bar layout, see [README.md](README.md).
