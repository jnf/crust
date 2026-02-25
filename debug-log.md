# Debug Log

Changes made to the Pi outside of the crust binary itself. AKA: Everything that went wrong.

---

## 2026-02-20 — I2C speed

**Change:** `/boot/firmware/config.txt`
```
dtparam=i2c_arm=on
dtparam=i2c_arm_baudrate=400000
```
Raised I2C to 400kHz fast-mode for lower display latency.

---

## 2026-02-21 — Fix MPD ALSA device (no audio from MPD)

**Problem:** `speaker-test` produced audio but MPD playback was silent.

**Root cause:** `hw:vc4hdmi,0` only accepts `IEC958_SUBFRAME_LE` (raw S/PDIF frames). MPD was sending standard PCM and ALSA rejected it silently. The `speaker-test` tool uses ALSA's `plughw` layer by default, which transparently converts PCM → IEC958, so it worked while MPD didn't.

**Fix:** `/etc/mpd.conf` — changed ALSA output device:
```
# before
device      "hw:vc4hdmi,0"

# after
device      "plughw:vc4hdmi,0"
```

`plughw` enables ALSA's plugin layer, which handles the PCM → IEC958 format conversion required by the vc4-hdmi driver.

Applied with:
```sh
sudo sed -i 's/device.*"hw:vc4hdmi,0"/device      "plughw:vc4hdmi,0"/' /etc/mpd.conf
sudo systemctl restart mpd
```

---

## 2026-02-21 — Bar layout tuning (X_OFFSET, BAR_STEP)

**Change:** `src/main.rs` constants.

The SSD1305 has 132 column drivers for a 128px panel; hardware columns 0–3 are off-screen on the left. A full `X_OFFSET=4` pushes bars right but clips bar 15 by 3px. `X_OFFSET=2` with `BAR_STEP=8` (7px bar + 1px gap) distributes the deficit: bar 0 loses 2px (shows 5px, flush with panel left edge), bar 15 loses 1px (shows 6px). All 14 inner bars render at full 7px width.

Note: avoid `bars=15` in CAVA — empirically, CAVA outputs 32 bytes per frame regardless (not 30), causing frame-boundary drift and scrolling. 16 bars is stable.

---

## 2026-02-21 — CAVA sensitivity tuning

**Change:** `~/.config/cava/config` — added `sensitivity = 50` (default is 100).

Halving sensitivity prevents bars from maxing out on moderately loud passages, giving more visible dynamic range.

---

## 2026-02-22 — Systemd auto-start services

**Problem:** After a power cycle, MPD gets "Unknown error 524" (~75s post-boot) because the vc4-hdmi ALSA driver requires the HDMI link to be fully negotiated first (~3m post-boot for the _terrible_ HDMI extractor I started with). cava and crust had no auto-start at all.

**Root cause of error 524:** The vc4-hdmi audio driver is coupled to the DRM display driver. Until the HDMI extractor completes link negotiation, the PCM device cannot be opened. `speaker-test` worked as a workaround because it ran after the link was up.

**Fix: three files deployed to `/etc/systemd/system/`**

`mpd.service.d/10-hdmi-wait.conf` — drop-in that blocks MPD start until the HDMI audio device successfully accepts a 1-second silent probe (establishes the link, then exits):
```ini
[Service]
ExecStartPre=/bin/bash -c 'until aplay -D plughw:vc4hdmi,0 -f S16_LE -r 44100 -c 2 -d 1 /dev/zero -q 2>/dev/null; do sleep 5; done'
TimeoutStartSec=600
```

`cava.service` — starts CAVA after MPD, restarts on failure.

`crust.service` — starts crust after CAVA; `ExecStartPre` waits for `/tmp/cava.fifo`
to exist as a named pipe before opening it.

Applied with:
```sh
sudo systemctl enable mpd cava crust
sudo systemctl start cava crust
```

Service files are version-controlled in the repo under `systemd/`.

---

## 2026-02-22 — Audio xrun fix (crust threading + MPD buffer)

**Problem:** Intermittent audio dropouts every few seconds. MPD logged:
`alsa_output: Decoder is too slow; playing silence to avoid xrun`

**Root cause:** crust's main loop was strictly sequential: `read_exact(cava.fifo)` →
render → `display.flush()` (I2C, ~15ms, uninterruptible D-state). While the I2C write blocked, nothing drained `cava.fifo`. Backpressure propagated: cava.fifo → CAVA → mpd.fifo → MPD output thread stall → xrun. Confirmed by stopping crust: zero xruns for 60s; restarting crust: xruns resumed immediately.

The ~25% `wa` in vmstat was caused by crust's frequent I2C D-state blocks, not SD card reads (diskstats showed near-zero block I/O during the same window).

**Fix 1 — `src/main.rs`:** moved FIFO read onto a dedicated thread so it drains
continuously regardless of I2C write duration. The render loop reads the latest frame from a shared `Arc<Mutex<[u8; 32]>>` and calls `display.flush()` independently.

**Fix 2 — `/etc/mpd.conf`:** increased `audio_buffer_size` from default 4 MB to 32 MB as a secondary safeguard against any remaining pipeline latency spikes.
```
audio_buffer_size    "32768"
```

Applied with:
```sh
sudo sed -i 's/^filesystem_charset.*"UTF-8"/filesystem_charset\t\t"UTF-8"\naudio_buffer_size\t\t"32768"/' /etc/mpd.conf
sudo systemctl restart mpd
# deploy new crust binary (built with cargo zigbuild --release)
sudo systemctl restart crust
```
