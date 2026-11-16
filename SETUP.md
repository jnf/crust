# crust — setup guide

Step-by-step instructions for a competent tinkerer setting up crust on a fresh (but already-booted) Raspberry Pi Zero 2W.

**Assumed:** SSH access works, Raspberry Pi OS Lite (64-bit, aarch64) is running.

---

## 1. Hardware you'll need

| Item | Notes |
|------|-------|
| Raspberry Pi Zero 2W | 64-bit OS required |
| [Adafruit OLED Bonnet 4567](https://www.adafruit.com/product/4567) | 128×32px, SSD1305, I2C — seat it on the GPIO header |
| Audio signal chain | crust drives a visualizer; it needs audio playing through MPD. This build outputs to a **NAD D3045 as a USB DAC** (it enumerates cleanly even behind a USB hub). Any setup that routes audio through MPD on the Pi will work. (An HDMI → extractor → S/PDIF path was retired in 2026-11; see debug-log.md for that history.) |

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

The MPD config is tracked in `config/mpd.conf` (the shared, deployable layer) and installed by copy. What's specific to your box (the music directory, your DAC's output, and optionally a LAN bind) lives in a separate `mpd_local.conf` that `mpd.conf` includes, so the tracked file stays generic to any box. mpd runs as Debian's unprivileged `mpd` user, never your login user, so make the music readable to it.

```sh
# 1. machine-specific settings (not tracked) — set music_directory and your DAC
sudo cp config/mpd_local.conf.example /etc/mpd_local.conf
sudoedit /etc/mpd_local.conf

# 2. let mpd (group audio) read the music: home traversable, music group-readable,
#    setgid so new albums inherit the group
chmod 711 ~
mkdir -p ~/music && chgrp audio ~/music && chmod 2750 ~/music

# 3. the shared config
sudo cp config/mpd.conf /etc/mpd.conf
sudo systemctl restart mpd
```

> **Set your DAC.** The example's output is `plughw:CARD=YOUR_CARD,DEV=0`. Run `aplay -l` to find your device and set `CARD=` to match (this build's NAD D3045 is `CARD=Audio`).

> **Note:** `plughw` (not bare `hw`) is required on the ALSA output. The bare `hw:` device exposes only the hardware's native formats (S32_LE on the NAD); `plughw` enables the ALSA plugin layer that transparently converts our 44100/16-bit PCM to whatever the endpoint accepts.

---

## 5. Configure CAVA

The CAVA config is tracked in `config/cava.conf` (no machine-specifics — it's all `/tmp` paths). Install by copy:

```sh
mkdir -p ~/.config/cava
cp config/cava.conf ~/.config/cava/config
```

> **Important:** keep `bars = 16`. CAVA always writes 32 bytes per frame regardless of the bars setting; other values cause frame-boundary drift.

---

## 6. Set up the MPD service drop-ins

One drop-in hardens MPD startup:

```sh
sudo mkdir -p /etc/systemd/system/mpd.service.d
sudo cp systemd/mpd.service.d/10-audio-wait.conf /etc/systemd/system/mpd.service.d/
sudo systemctl daemon-reload
```

**`10-audio-wait.conf`** — MPD opening an ALSA output before the device is ready gets "Unknown error 524" and silently fails. This `ExecStartPre` loops a 1-second silent `aplay` probe against the NAD every 5 seconds until it accepts, then unblocks MPD. The NAD is ready in under a second, so the timeout is 60s; a DAC that hasn't shown up by then is unplugged, and failing loudly beats waiting. Needs `alsa-utils` for `aplay`.

> **Mask `mpd.socket`.** Debian's `mpd.service` carries `Also=mpd.socket`, so enabling the service enables socket activation too, which holds port 6600 itself and makes mpd ignore `bind_to_address`. mpd starts at boot anyway, so the socket buys nothing: `sudo systemctl mask mpd.socket`.

---

## 7. Install systemd services

```sh
sudo cp systemd/cava.service  /etc/systemd/system/
sudo cp systemd/crust.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable mpd cava crust
```

All three services will start automatically on boot in the correct order: MPD (after the audio device is ready) → CAVA → crust.

> **Edit for your user.** Unlike the configs, the unit files carry concrete paths — `User=jey`, `ExecStart=/home/jey/crust`, and CAVA's `-p /home/jey/.config/cava/config`. There's no clean include seam for systemd units, so change these to your user before copying them.

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

**Restart the visualizer (cava + crust) on its own:**

```sh
scripts/start.sh   # = sudo systemctl restart cava crust
```

> **When you need this:** CAVA reads `/tmp/mpd.fifo` and writes `/tmp/cava.fifo`; crust reads the latter. CAVA must re-open both pipes whenever they're recreated, or it emits silence and the OLED goes blank. The cava/crust units are `PartOf` MPD's restart cascade (see `systemd/cava.service`), so a `systemctl restart mpd` **re-syncs the visualizer automatically** — you don't need this command for that. Reach for `start.sh` when you bounce the visualizer *without* restarting MPD (e.g. after editing CAVA's config). Switching outputs with `mpc enable/disable` doesn't restart MPD at all, so it needs no re-sync.

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
