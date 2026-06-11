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

**Root cause:** crust's main loop was strictly sequential: `read_exact(cava.fifo)` → render → `display.flush()` (I2C, ~15ms, uninterruptible D-state). While the I2C write blocked, nothing drained `cava.fifo`. Backpressure propagated: cava.fifo → CAVA → mpd.fifo → MPD output thread stall → xrun. Confirmed by stopping crust: zero xruns for 60s; restarting crust: xruns resumed immediately.

The ~25% `wa` in vmstat was caused by crust's frequent I2C D-state blocks, not SD card reads (diskstats showed near-zero block I/O during the same window).

**Fix 1 — `src/main.rs`:** moved FIFO read onto a dedicated thread so it drains continuously regardless of I2C write duration. The render loop reads the latest frame from a shared `Arc<Mutex<[u8; 32]>>` and calls `display.flush()` independently.

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

---

## 2026-04-20 — HPD re-negotiation fix (HDMI extractor replacement)

**Problem:** Replaced the original HDMI extractor with an OREI BK-41A (4-port HDMI switch/extractor). Intermittent audio stuttering followed — much worse immediately after switching the OREI back to the Pi's input, improving over ~10 seconds, then settling to occasional intermittent glitches. MPD logged nothing; no xrun reports.

**Root cause:** The OREI BK-41A drops HPD (Hot Plug Detect) on inputs that aren't currently selected. When the Pi's input is deselected, the vc4-hdmi driver sees a disconnect and tears down the HDMI link. When the Pi's input is re-selected, the Pi must re-negotiate from scratch: re-read EDID, re-establish mode, re-initialize audio output. This takes several seconds. The initial heavy stuttering is the Pi completing re-negotiation; the ~10s improvement window is the OREI's audio clock recovery re-locking onto the stabilized signal.

**Fix:** `/boot/firmware/config.txt` — add `hdmi_force_hotplug=1`. This makes the Pi treat HDMI as always connected regardless of HPD state, maintaining a stable output continuously. When the OREI switches back, it finds the Pi already running instead of cold-starting.

```
hdmi_force_hotplug=1
```

**Diagnostic dead end — IEC958 AES3:** The IEC958 Playback Default control (`amixer -c 0 cget numid=4`) shows `AES3=0x01` ("sample rate not indicated"). This is vc4-hdmi driver behavior; `amixer cset` writes are silently ignored — the driver owns this control and resets it continuously. It is not the cause of the stuttering and cannot be fixed from userspace.

**Also discovered:** `hdmi_group` and `hdmi_mode` in `config.txt` are silently ignored under `dtoverlay=vc4-kms-v3d` + `disable_fw_kms_setup=1`. The full KMS driver determines output mode from EDID negotiation. To force a specific mode with full KMS, use a `video=` kernel parameter in `/boot/firmware/cmdline.txt` instead — e.g. `video=HDMI-A-1:1280x720@60`.

---

## 2026-06-10 — crust I2C interrupt storm starving MPD (decoder too slow, round 2)

**Problem:** Audible audio stutter/dropout during playback. Suspected not classic xruns — and right: MPD logged `alsa_output: Decoder is too slow; playing silence to avoid xrun`, but only every 3–5 minutes, not the every-few-seconds pattern of the 2026-02-22 xrun bug.

**Profiling (`bars.local`, playing):** Load 1.29/4 cores, 68% idle — not CPU-bound. But ~22% `wa` (iowait) with `vmstat` `bi/bo = 0` (no disk I/O), and `b` (blocked tasks) pinned at 1. The blocked task was crust, in `D` (uninterruptible sleep) inside `bcm2835_i2c_xfer`. `/proc/interrupts` showed the I2C controller as the dominant interrupt source on the whole system by 3× — **~7,000 interrupts/sec**, ~80% of all interrupts.

**Root cause:** The 2026-02-22 fix put the FIFO read on its own thread so a slow I2C flush couldn't back up the CAVA → MPD pipeline — but it left the *render* loop free-running. The loop redrew and `flush()`ed the full OLED framebuffer as fast as flush returned (~60fps), re-pushing frames over I2C even when CAVA (at ~30fps) had produced nothing new, and even during silence when the bars didn't move. The bcm2835 I2C driver is interrupt-driven, so this made I2C the dominant IRQ. That interrupt load (plus the IPI "function call interrupts" it triggers) periodically preempted MPD's decoder thread — which runs as plain SCHED_OTHER with no priority — long enough to miss the output deadline and insert silence. The 2026-02-22 fix solved FIFO-read backpressure; this is the *other* half — the render loop itself flooding the bus.

> The ~22% `wa` was a red herring twice over: not disk (bi/bo=0), and the aggregate didn't track the fix (multicore iowait attributes a blocked task's wait to whatever core is idle). The honest signal was per-task `D`/`S` sampling.

**Fix — `src/main.rs`, two phases** (full design + verification in `plans/block-render-on-new-data.md`):

1. **Block on new data.** Replaced the `Arc<Mutex<[u8;32]>>` shared buffer with an `mpsc` channel. The reader thread `send()`s each frame (unbounded — never blocks, so it keeps draining `cava.fifo`); the render loop blocks on `recv()` until a fresh frame arrives, draining to the latest to avoid lagging the audio. Render now tracks CAVA's ~30fps instead of free-running.
2. **Dirty check.** Quantize each frame to its bar pixel heights and skip the draw + `flush()` entirely when they match the last flushed heights. During silence or held notes, I2C traffic drops to zero.

**Measured (same track playing):**

| | I2C int/sec, playing | I2C int/sec, silent | crust `D`-time | crust CPU |
|---|---|---|---|---|
| Before | ~6,989 | ~6,989 | ~94% | 8.3% |
| Phase 1 | ~2,800 | ~2,800 | ~30% | 3.6% |
| Phase 2 | ~2,300–2,500 | **0** | ~30% / 0 silent | ≤3.6% |

**Verification:** A 15-minute untouched continuous-playback soak logged **zero** "decoder too slow" events (baseline cadence: one every 3–5 min). A first attempt using a `journalctl --since "10 min ago"` lookback falsely read clean — that window spanned the old binary and the deploy, not the new binary's uptime; the real soak is what confirmed the fix.

```sh
# deploy (cross-built on dev machine with cargo zigbuild --release)
sudo systemctl stop crust
scp target/aarch64-unknown-linux-gnu/release/crust jey@bars.local:~/crust
sudo systemctl start crust
```

**Parked (not done):** Giving MPD RT scheduling priority (or negative nice) would harden the audio thread against any future runaway — belt-and-suspenders, since the interrupt-storm reduction alone resolved the starvation here.

---

## 2026-06-11 — Drop the HDMI extractor; drive the NAD D3045 directly as a USB DAC

**Change:** Retire the entire `HDMI → OREI BK-41A → S/PDIF` output segment and play straight to the NAD D3045 over USB. The OREI only ever existed to pull audio off the Pi's HDMI port; the NAD has a USB input, so the extractor is dead weight. The HDMI path is kept as a configured, disabled fallback. Visualizer (FIFO → CAVA → crust) is untouched — it's independent of the playback device.

**The feared snag that wasn't:** The NAD lives behind a USB hub (Genesys Logic, `05e3:0610`). Worry was that the Pi wouldn't enumerate a DAC chained behind a hub. It does, cleanly — `lsusb` shows both, ALSA exposes the NAD as `card 0 [Audio]` at `usb-3f980000.usb-1.3`, and a silent probe opened it at our exact format on the first try:
```sh
aplay -D plughw:CARD=Audio,DEV=0 -f S16_LE -r 44100 -c 2 -d 1 /dev/zero -q   # PROBE_OK
```
`/proc/asound/card0/stream0` confirms a real async DAC: S16/S32_LE, 44.1k–384k. Async feedback later measured locked at 44103 Hz during playback.

**Fix 1 — `/etc/mpd.conf`:** NAD primary, HDMI kept but disabled. Note `CARD=Audio` (the NAD's ALSA id), not `hw:0` — resilient to card-number reshuffles.
```
audio_output {
    type        "alsa"
    name        "NAD USB DAC"
    device      "plughw:CARD=Audio,DEV=0"   # plughw still required: DAC's native EP is S32_LE
    mixer_type  "software"
}
audio_output {
    type        "alsa"
    name        "HDMI Audio"
    device      "plughw:vc4hdmi,0"
    mixer_type  "software"
    enabled     "no"                          # fallback; mpc enable to switch
}
```
Switching is runtime, no restart: `mpc disable "NAD USB DAC" && mpc enable "HDMI Audio"`. Enable-state persists in MPD's state file across reboots (verified).

**Fix 2 — `10-hdmi-wait.conf` → `10-audio-wait.conf`:** The wait drop-in (added 2026-02-22) was hard-looping `aplay` against `plughw:vc4hdmi,0` — which is now disconnected, since this is a headless unit and the micro-HDMI is unplugged. **This was actively bricking the box:** MPD stuck in `activating`, looping the dead HDMI probe until the 600s timeout. Repointed to probe **NAD-or-HDMI**, unblocking on whichever is ready first, so both the USB path and an HDMI-only fallback boot correctly:
```ini
ExecStartPre=/bin/bash -c 'until aplay -D plughw:CARD=Audio,DEV=0 ... || aplay -D plughw:vc4hdmi,0 ...; do sleep 5; done'
TimeoutStartSec=600   # generous margin retained for HDMI's ~3min cold-boot negotiation
```

**Fix 3 — `20-iec958.conf` hardened (the landmine):** Sometime after 2026-04-20 the AES3 "dead end" got a workaround — an `ExecStartPost` that re-asserts `AES3=0x00` *after* MPD opens the device (MPD's ALSA plugin resets it to `0x01` on open; the post-open write sticks, unlike the pre-open writes that 2026-04-20 found were swallowed). That drop-in was hardcoded to `amixer -c 0`. **Card 0 is now the NAD**, which has no `numid=4` IEC958 control — so the `cset` would fail, and a failed `ExecStartPost` takes the whole service down with it. Hardened to address HDMI by its stable card *id*, guard on the control existing, and always exit 0:
```ini
ExecStartPost=/bin/bash -c 'sleep 2; amixer -c vc4hdmi cget numid=4 >/dev/null 2>&1 && amixer -c vc4hdmi cset numid=4 AES0=0x04,AES1=0x00,AES2=0x00,AES3=0x00 >/dev/null 2>&1; exit 0'
```
Now a no-op on the USB path, still corrects the OREI's S/PDIF clock byte on the fallback path.

**The instructive bug — restarting MPD blanks the OLED:** After switching configs, audio played but the display stayed dark. cava and crust both `active`; not a crash. The chain: restarting MPD unlinks and recreates `/tmp/mpd.fifo`, and CAVA — which opened the read end at *its* boot — ends up emitting **silence**. crust faithfully renders nothing (and with the 2026-06-10 dirty-check, skips the flush entirely → 0% CPU, looks hung but isn't).

Diagnosis, not guess:
```sh
timeout 1 cat /tmp/cava.fifo | od -An -tu2   # all zeros → cava is emitting silence
ps -o stat,%cpu -C crust                     # S, 0.0% — parked on dirty-check, not drawing
```
After a forced `restart cava → restart crust` (in that order — crust must reopen CAVA's *fresh* `cava.fifo`), cava emitted real bar values and crust went to `D`-state I2C writes. This is what `scripts/start.sh` (`systemctl restart cava crust`) was already for.

**Fix 4 — make the re-sync automatic (`PartOf`):** Bouncing cava/crust by hand after every MPD restart is a footgun. systemd splits unit relationships into two orthogonal axes — *dependency* (`Wants`/`Requires`/`BindsTo`/`PartOf`) and *ordering* (`After`/`Before`) — and the units had ordering (`After=`) and boot-coupling (`Wants=`) but no restart cascade. `PartOf` is the precise tool: **one-directional, restart/stop-only propagation.** Added `PartOf=mpd.service` to `cava.service` and `PartOf=cava.service` to `crust.service`. Now `systemctl restart mpd` cascades `mpd → cava → crust` in `After=` order, and the visualizer re-syncs itself. `start.sh` survives for bouncing cava/crust *without* restarting MPD (e.g. after editing CAVA's config).

**Verification:**
- *Runtime cascade:* `systemctl restart mpd` alone cycled all three PIDs; cava output went non-zero with no manual step.
- *Cold boot (the real test):* `sudo reboot`. MPD `activating → active` ~10s after the Pi came up (NAD probe, **0× error 524**), services all active, MPD auto-resumed playback, NAD PCM `RUNNING`, cava feeding crust, output enable-state (NAD on / HDMI off) intact.

```sh
# all deployed to the Pi and committed to systemd/ + documented in SETUP.md/README.md
# canon: 60fe3a7 (USB DAC) → dc53405 (PartOf) → db3ead9 (docs), pushed to origin
```

**Fallback procedure (if the USB DAC ever drops out):** reconnect the micro-HDMI/OREI, then `mpc disable "NAD USB DAC" && mpc enable "HDMI Audio"`. The `20-iec958.conf` post-open AES3 fix fires automatically on the HDMI path; no reboot needed.
