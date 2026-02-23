# Pi configuration log

Changes made to the Pi outside of the crust binary itself.

---

## 2026-02-21 — Fix MPD ALSA device (no audio from MPD)

**Problem:** `speaker-test` produced audio but MPD playback was silent.

**Root cause:** `hw:vc4hdmi,0` only accepts `IEC958_SUBFRAME_LE` (raw S/PDIF frames).
MPD was sending standard PCM and ALSA rejected it silently. The `speaker-test` tool
uses ALSA's `plughw` layer by default, which transparently converts PCM → IEC958 —
hence it worked while MPD didn't.

**Fix:** `/etc/mpd.conf` — changed ALSA output device:
```
# before
device      "hw:vc4hdmi,0"

# after
device      "plughw:vc4hdmi,0"
```

`plughw` enables ALSA's plugin layer, which handles the PCM → IEC958 format
conversion required by the vc4-hdmi driver.

Applied with:
```sh
sudo sed -i 's/device.*"hw:vc4hdmi,0"/device      "plughw:vc4hdmi,0"/' /etc/mpd.conf
sudo systemctl restart mpd
```

---

## 2026-02-21 — Bar layout tuning (X_OFFSET, BAR_STEP)

**Change:** `src/main.rs` constants.

The SSD1305 has 132 column drivers for a 128px panel; hardware columns 0–3 are
off-screen on the left. A full `X_OFFSET=4` pushes bars right but clips bar 15
by 3px. `X_OFFSET=2` with `BAR_STEP=8` (7px bar + 1px gap) distributes the
deficit: bar 0 loses 2px (shows 5px, flush with panel left edge), bar 15 loses
1px (shows 6px). All 14 inner bars render at full 7px width.

Note: avoid `bars=15` in CAVA — empirically, CAVA outputs 32 bytes per frame
regardless (not 30), causing frame-boundary drift and scrolling. 16 bars is stable.

---

## 2026-02-21 — CAVA sensitivity tuning

**Change:** `~/.config/cava/config` — added `sensitivity = 50` (default is 100).

Halving sensitivity prevents bars from maxing out on moderately loud passages,
giving more visible dynamic range.

---

## 2026-02-22 — Systemd auto-start services

**Problem:** After a power cycle, MPD gets "Unknown error 524" (~75s post-boot) because
the vc4-hdmi ALSA driver requires the HDMI link to be fully negotiated first (~187s
post-boot for this HDMI extractor). cava and crust had no auto-start at all.

**Root cause of error 524:** The vc4-hdmi audio driver is coupled to the DRM display
driver. Until the HDMI extractor completes link negotiation, the PCM device cannot be
opened. `speaker-test` worked as a workaround because it ran after the link was up.

**Fix: three files deployed to `/etc/systemd/system/`**

`mpd.service.d/10-hdmi-wait.conf` — drop-in that blocks MPD start until the HDMI audio
device successfully accepts a 1-second silent probe (establishes the link, then exits):
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

## 2026-02-22 — MPD audio buffer (dropout fix)

**Problem:** Intermittent audio dropouts every few seconds. MPD logged:
`alsa_output: Decoder is too slow; playing silence to avoid xrun`

**Root cause:** Sustained ~25% iowait from the SD card causes the decoder thread to stall.
With the default 4 MB audio buffer, the output runs dry before the decoder recovers.

**Fix:** `/etc/mpd.conf` — added:
```
audio_buffer_size    "32768"
```
32 MB gives ~185 seconds of 44100/16/2 headroom, absorbing SD card latency spikes.

Applied with:
```sh
sudo sed -i 's/^filesystem_charset.*"UTF-8"/filesystem_charset\t\t"UTF-8"\naudio_buffer_size\t\t"32768"/' /etc/mpd.conf
sudo systemctl restart mpd
```

---

## 2026-02-20 — I2C speed

**Change:** `/boot/firmware/config.txt`
```
dtparam=i2c_arm=on
dtparam=i2c_arm_baudrate=400000
```
Raised I2C to 400kHz fast-mode for lower display latency.
