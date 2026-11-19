# Debug Log

Changes made to the Pi outside of the crust binary itself. AKA: Everything that went wrong.

As of 2026-11-15, the jukebox-specific pieces (`bars-output`, its udev rule, `mout`, the playback outputs in `mpd_local.conf`) live in bars, a private repo that layers on crust, and their story continues in bars' integration log. The entries below that mention them stay as they were.

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

---

## 2026-11-09 — Rebuilt jukebox: HDMI retired, mpd off the login user, socket masked

**Context:** New SD card, DietPi on Debian 13 Trixie, hostname `jukebox` (the system-level story lives in bars' `provisioning.md`). Same Pi Zero 2W, same NAD D3045 over USB. This entry is everything that changed in crust's tracked config and units on the way to first sound.

**Change 1 — HDMI is gone** (`cac84d0`, `f9c7162`). The 2026-06-11 fallback carried the most fragile config on the box: the dual probe, the 600s timeout sized for HDMI's three-minute handshake, and `20-iec958.conf`, which a card-index shuffle nearly turned into an mpd-killer. Now:
- `mpd.conf` has one ALSA output, the NAD.
- `10-audio-wait.conf` probes the NAD only, with `TimeoutStartSec=60`. The NAD opens in under a second, and a DAC still missing after a minute is unplugged. Fail loudly.
- `20-iec958.conf` is deleted. The fallback procedure in the 2026-06-11 entry is dead.
- The board backs this up: DietPi ships `dtparam=audio=off` and the KMS overlay commented out, so HDMI audio doesn't even initialize.

**Change 2 — mpd runs as `mpd`, not `jey`** (`c096a62`). `mpd_local.conf` used to set `user "jey"`. The new box made the problem visible: mpd listens on the LAN with no password, and `jey` has passwordless sudo. An mpd exploit would be one hop from root. The trap: `mpd.service` has no `User=`. Debian's stock `/etc/mpd.conf` supplies `user "mpd"`, and our tracked file replaces it wholesale, so dropping `user` from the local layer would have left mpd running as **root**. The tracked `mpd.conf` now sets `user "mpd"` itself. The music has to be readable by group `audio`:
```sh
chmod 711 ~                                                     # traverse only
mkdir -p ~/music && chgrp audio ~/music && chmod 2750 ~/music   # setgid: albums inherit audio
sudo -u mpd ls ~/music                                          # verify
```

**Change 3 — `mpd.socket` masked** (`f9c7162`). Debian's `mpd.service` carries `Also=mpd.socket`, so `systemctl enable mpd` quietly turns on socket activation, which holds 6600 itself and overrides `bind_to_address`. This is the override `3eaff53`'s comment warned about. mpd starts at boot anyway, so the socket buys nothing.
```sh
sudo systemctl mask mpd.socket
sudo systemctl enable mpd cava    # "Unit /etc/systemd/system/mpd.socket is masked, ignoring."
```

**Change 4 — `alsa-utils` is a dependency.** DietPi doesn't ship `aplay`, and `10-audio-wait.conf` loops on it. Without the package, the probe fails every iteration and mpd times out. `apt install alsa-utils`.

**Red herrings in the journal:**
- `bind to '0.0.0.0:6600' failed ... Address already in use`: mpd binds `[::]:6600` first, that dual-stack socket also covers IPv4, and the second bind trips over the first. mpd continues, and IPv4 clients connect fine.
- `Referenced but unset environment variable ... MPDCONF`: Debian's unit reads it from `/etc/default/mpd`, which doesn't exist here. mpd falls back to `/etc/mpd.conf`.

**Red herring outside the journal:** the first play produced no sound, while everything on the box said audio was flowing (PCM `RUNNING`, `avail` moving, the NAD's clock valid, no mixer controls to mute). The NAD was on another input. It accepts a USB stream either way.

**Verification** (Pale Jay, *Low End Love Songs*, `mpc volume 20`):
```sh
ps -T -o spid,cls,rtprio,comm -p $(pgrep -x mpd)   # rtio, output:NAD USB, output:CAVA FIF → FF 40
cat /proc/asound/card0/pcm0p/sub0/hw_params        # S32_LE, 44100 — plughw converting
timeout 1 cat /tmp/cava.fifo | od -An -tu2         # non-zero bars
```
- `LimitRTPRIO=40` takes: both outputs run SCHED_FIFO 40.
- Don't parse `/proc/*/stat` for this. `output:NAD USB` has a space in its name, which shifts the fields, and the first read came back as garbage.
- 342Mi available with mpd and cava playing, against a 394Mi floor on the bare box.
- crust itself isn't deployed yet, so the OLED is still dark. That's the next entry.

---

## 2026-11-09 — ~125ms of silence at song borders (known, accepted)

**Symptom:** `alsa_output: Decoder is too slow; playing silence to avoid xrun`, once at most track changes, never mid-song. Shows up before crust is even deployed.

**Not the cause:** disk (the album sat in page cache, zero blocks read), format changes (every track 2ch/44.1k/320k), crust, or the ALSA buffer size.

**Root cause** (read from mpd 0.24.4 source, unchanged since 0.23.12): the player won't cross a song border until *every* output has consumed the old song. The FIFO output paces itself to the wall clock (`Timer` + `WaitForDelay`) with about one 23ms chunk of lead, so it finishes the old song at almost the instant it's heard. The NAD's ALSA output finished ~0.5s earlier and plays its buffer dry while it waits, then pads one period (5513 frames, ~125ms) of silence. A bigger ALSA buffer just drains the same way.

**Proof** (seek to 8s before a track end, count the warning across 3 borders):

| Second output | cava | Stalls |
|---|---|---|
| CAVA FIFO | reading | 3/3 (6/6 earlier) |
| CAVA FIFO | stopped | 0/3 |
| FIFO disabled | — | 0/3 |
| `null`, `sync "yes"` | — | 3/3 |
| `null`, `sync "no"` | — | 0/3 |

The red herring inside the proof: with cava stopped the FIFO fills, `Play()` hits `EAGAIN`, and `Cancel()` drains the pipe *and resets the timer*, which throws the output ahead of the wall clock. So the first "restored" control came back clean, and only the rerun with cava reading went red.

**Decision:** live with it. The gap mostly lands in the silence between songs; it costs gapless albums a ~125ms hole. The fix, if it's ever worth it, is an ALSA loopback (`snd-aloop`) feeding cava, so both outputs buffer ahead the same way. A `pipe` output is out: its writes block, so a stalled cava would stall playback at the next border. Upstream's version of this is probably MPD #2250 (unresolved); this table would make a good comment there.

**Addendum, 2026-11-10 (the Phase 8 soak):** the warning fires on *natural* track ends only. The soak had 19 player changes: four `next` (three a second apart), a `prev`, a seek, a pause/resume, and two clear-and-play. It logged four warnings, each in the same second as one of the four natural borders, and none for any scripted change. That fits the root cause. A skip cancels every output at once, so the ALSA output never has a buffer to play dry. Only a natural border makes it wait on the FIFO. "Once at most track changes" above should read "once at most *natural* track changes."

---

## 2026-11-09 -- Sennheiser MOMENTUM 3 over USB: compatible, cheap, sounds great (experiment, reverted)

**Why:** the NAD was upstairs during the 512G re-flash, and the cans are the realistic way to listen when jey is away from the main setup. Bluetooth was ruled out: it shares the Zero 2W's radio with Wi-Fi (the jukebox's only link), re-encodes lossily, and needs a stack Phase 2 deliberately doesn't carry.

**The device:** `card 0 [M3]`, full-speed USB through a hub. One playback format only, **S16_LE, 2ch, 48000**. One mixer control, `PCM`, whose dB readout is nonsense (40% reported as +18dB). So every 44.1k track resamples, and 24-bit sources would be cut down to 16-bit.

**The overrides** (box-local, both removed afterwards):
- `/etc/mpd_local.conf` gained a second `audio_output`: `alsa`, `plughw:CARD=M3,DEV=0`, `format "48000:16:2"`. The `format` line makes mpd resample with its own libsamplerate ("Fastest Sinc"); leaving it out would hand the job to ALSA plug's linear converter instead.
- `/etc/systemd/system/mpd.service.d/20-cans.conf`: an empty `ExecStartPre=` to clear the tracked wait, then the same probe loop aimed at `CARD=M3` at 48k. Without it, mpd waits the full 60s for the NAD and fails.

**Verification** (Azuah, *Dreams EP*, 320k MP3 at 44.1k, idle box, PCM 40%):
- `hw_params`: S16_LE, 48000 -- mpd resampled before ALSA.
- `output:Sennheiser` thread **~10% of one core** (the resampler); decoder ~10%. All three outputs SCHED_FIFO 40.
- 349Mi available, mpd RSS 99Mi -- level with the NAD's 342Mi.
- With the NAD absent, mpd logs one `Failed to open "NAD USB DAC"` at play and carries on with the other outputs.
- cava FIFO carries non-zero bars; the OLED ran.
- Song borders weren't tested.
- By ear (jey): "really clean... especially the low end... there's _space_ in there."

**Side finding, the iowait:** the ~22-25% iowait with one blocked task and no disk I/O showed up again here. crust sits in `D` inside `bcm2835_i2c_xfer` most of the time, but it isn't the cause: with crust stopped, iowait held at 25% and the blocked task stayed. Interrupts fell from ~19k/s to ~15.5k/s, which puts crust's I2C load at about 3.5k/s. Still unexplained; carried to the Phase 8 soak. **Solved 2026-11-10:** no task is waiting at all. A kernel counter is stuck at 1 on CPU 3. See bars' `integration-log.md`.

**Revert:** restored `mpd_local.conf` from a copy taken first, removed `20-cans.conf`, `daemon-reload`. The wait is back on `CARD=Audio`.

**If the cans become a regular path** (trigger: the second time jey reaches for them): track both outputs, make the wait accept either card, and switch with `mpc enable`/`disable`. That's a config and contracts-table change, not an override.

---

## 2026-11-11 -- `stop.sh` can't find `i2ctransfer` on the rebuilt jukebox

**Symptom:** `stop.sh` stops mpd, cava and crust, then fails on its last line with `i2ctransfer: command not found`, so the OLED keeps the last frame lit. jey hit it on 2026-11-10; it's the only way to clear the OLED right now.

**Not the cause:** a missing package. `i2c-tools` 4.4-2 is installed; DietPi's `dietpi-set_hardware i2c enable` (provisioning Phase 3) pulls it in.

**Root cause:** Debian installs `i2cdetect`, `i2ctransfer` and friends to `/usr/sbin`, and a non-root user's PATH (`/usr/local/bin:/usr/bin:/bin:/usr/local/games:/usr/games`) doesn't include it, in a login shell or otherwise. The old box hid this somehow (unverified how).

**Fix:** `stop.sh` calls `/usr/sbin/i2ctransfer` by full path. No sudo: jey is in the `i2c` group, which owns `/dev/i2c-1`. Adding `/usr/sbin` to jey's PATH would also work, but it's box config to remember for one script. Redeployed: `scp scripts/stop.sh jukebox.local:~/stop.sh`.

**Verified:** ran the exact line as jey, no sudo, mid-playback: `0xAE` blanked the OLED, and `0xAF` two seconds later brought it back with crust still drawing (jey watched the panel on a second run). The full `stop.sh` ran clean (exit 0) later that day, at the end of the cava trials.

**Side note:** with no `i2c-tools` on PATH, the same ioctl works from the perl in the base system, which is how the inverse-display check (`0xA7`/`0xA6`) ran the same day: `perl -e 'open(my $f,"+<","/dev/i2c-1") or die; ioctl($f,0x0703,0x3c) or die; syswrite($f,"\x00\xa7")'`. `0x0703` is `I2C_SLAVE`.

---

## 2026-11-11 -- cava writes zeros after a long stop (found 2026-11-12)

*Found: an integer overflow in cava's framerate estimate, set off by how long the player is stopped. The NAD lead below was a red herring. See 2026-11-12, "cava zeros: a 32-bit overflow after ~15 minutes of quiet".*

**Symptom:** after the player has been stopped or paused for a while, `mpc play` brings the sound back but the OLED stays dark. cava is writing all-zero frames to `/tmp/cava.fifo`. `systemctl restart cava` fixes it. First seen 2026-11-10: the queue ran out at 10:55, the player sat stopped about 25 minutes, and on resume mpd was feeding cava real audio (173KB/s, peak 32262) while cava wrote zeros. cava's write end on `/tmp/cava.fifo` had been reopened at 11:10, mid-stop.

**Rates (jey, on the NAD):** about 1 in 4 after a pause of 5+ minutes, and about 1 in 2 after a natural queue end (~30 minutes stopped).

**Ruled out:** a crust restart, and a 30s crust stop/start (both were run as controls 2026-11-10 and came back clean). The iowait control wedging cava was the first suspicion, and it was wrong.

**The probe:** `scripts/cava-probe.sh`, piped over ssh (`ssh jukebox.local bash -s < scripts/cava-probe.sh`), so nothing lands on the box. Run it in the dark state *before* restarting cava. It separates three hypotheses:

| Hypothesis | What it would show |
|---|---|
| Input stuck | cava's `rchar` flat: it stopped reading `/tmp/mpd.fifo` (mpd's fifo output drains its own pipe, so playback never stutters) |
| Stale pipe | cava's read fd points at an inode other than `/tmp/mpd.fifo` |
| Processing | `rchar` climbing ~176KB/s on the live inode, output still zero (autosens drifting through the silence is the unverified suspect) |

Both controls pass. Stopped: `rchar +0`, 0/32 nonzero, and both fds on the live inodes. Playing: `rchar +181080`/s (44100 × 4 bytes), 32/32 nonzero. The stopped control caught a probe bug: `od` collapses repeated lines into `*`, so all zeros counted as one nonzero value. `od -v` fixed it.

**Side finding:** while mpd is stopped, cava keeps writing zero frames at ~27fps (`wchar +864`/s). crust never loses its feed. It also means **"zeros for N seconds" is exactly what this bug looks like**, so idle detection must never be built on it (bars `plans/two-node.md`, Phase 9).

**Trials on the Sennheiser cans: 0 bad in 9.** One full-album natural end, then eight shortened ones (play the last track, seek to 97%, let the queue end naturally, hold 5 minutes stopped, `mpc play`, probe at +5s and +30s). Every probe read 32/32 nonzero and ~177KB/s in. At jey's 50% rate, nine clean in a row is about 1 in 500, so the setup differs in a way that matters.

**The lead: the NAD dropping off USB.** jey's trials ran on the NAD; these ran on the cans. tspine's field notes already record that the NAD in standby isn't USB-enumerated at all. Longer idle failing more often fits an auto-standby timer somewhere between 5 and 30 minutes (the D3045's timer is unverified). The cans never leave the bus. The chain from "the NAD re-enumerates" to "cava writes zeros while reading real audio" is unexplained; the 11-10 mid-stop reopen of `cava.fifo` might be part of it.

**Next experiment (on the NAD):** stop, and wait until the NAD actually leaves `/proc/asound/cards`, rather than trusting a timer. Then `mpc play` and probe. Add the kernel log's USB disconnects and re-enumerations during the hold to the probe's output.

---

## 2026-11-11 -- One output per card, and mpd's state file only ever disables

**Change:** bars Phase 2 (output targets). `mpd.conf` has a tracked `audio_output` per known card (`NAD` = `CARD=Audio`; `Cans` = `CARD=M3`, `format "48000:16:2"`). The boot wait (`10-audio-wait.conf`) probes every card mpd.conf names and starts mpd after 60s regardless, then runs `bars-output auto` (`ExecStartPost`). `bars-output` is at `/usr/local/bin`; jey's choice lives in `~/.config/bars/output`. This replaced the morning's hand-edited `CARD=M3` retarget of the tracked files.

**Deploy:**
```sh
scp config/mpd.conf systemd/mpd.service.d/10-audio-wait.conf scripts/bars-output jukebox.local:tmp/
ssh jukebox.local 'sudo install -m 644 tmp/mpd.conf /etc/mpd.conf
  sudo install -m 644 tmp/10-audio-wait.conf /etc/systemd/system/mpd.service.d/
  sudo install -m 755 tmp/bars-output /usr/local/bin/
  rm tmp/mpd.conf tmp/10-audio-wait.conf tmp/bars-output
  sudo systemctl daemon-reload && sudo systemctl restart mpd'
```

**No card at boot -- passes.** Rebooted with nothing on the USB port: the wait logged `no audio card after 60s; starting mpd anyway`, and mpd, cava and crust all came up `active`. The old NAD-only wait failed all three.

**The trap:** the first draft started `Cans` with `enabled "no"`. After the reboot, both playback outputs came back disabled, even though the state file said `audio_device_state:1:Cans`. With nothing enabled, mpd plays to the CAVA FIFO alone: silence, with the OLED dancing.

**Root cause** (mpd 0.24.4, `src/output/State.cxx`, `audio_output_state_read`): a stored `1` is an explicit no-op ("state is enabled: no-op"), and only a stored `0` acts, by disabling the output. Restore assumes every output starts enabled. `enabled "no"` breaks that assumption, so the output can never come back on across a restart. A red herring on the way: the first restart after the fix still showed `Cans` disabled, because mpd writes the state file at *stop*, from runtime state the bad boot had already left disabled.

**Fix:** every output starts enabled (no `enabled` lines), and `bars-output auto` runs once mpd is ready to settle on exactly one.

**Verified:** enabled `Cans` by hand, restarted mpd: the state file kept `1:Cans` and `Cans` came back enabled.

**Addendum, same evening -- mpd never retries a failed output.** Cans enabled, unplugged, *End* playing (to the FIFO alone; mpd logged `Failed to open "Cans" ... No such device`). Plugged the cans in: 60s later their PCM (`/proc/asound/M3/pcm0p/sub0/status`) was still `closed`, and mpd logged no second attempt. `mpc disable Cans; mpc enable Cans` didn't open it either. `mpc pause; mpc play` did, at once (`state: RUNNING`), at the expected volume. So bars-output gained a kick: while mpd is playing, if the chosen card's PCM is `closed`, pause and resume. A udev rule (`config/90-bars-output.rules` → `/etc/udev/rules.d/`) runs `bars-output auto` 2s after any sound card appears, via `systemd-run`, so udev never waits on it.

**Live test, same evening -- unplug and replug the cans mid-song.** The chain works: udev → `systemd-run` timer → `bars-output auto` → "kicked playback to open Cans" → `RUNNING`. The first run took **29s** from plug-in to sound, because a transient timer's `AccuracySec` defaults to 1min and systemd coalesced the 2s timer into a later wakeup. With `--timer-property=AccuracySec=100ms`, the second run took **3.4s** (timer +2.1s, kick +3.2s, PCM running +3.4s).

---

## 2026-11-12 -- mpd plays into the void when the NAD leaves mid-song

**Setting:** first boot in jey's office, on the NAD through jey's USB switch. The switch is itself a hub (GenesysLogic `05e3:0610` at `1-1`): the NAD sits at `1-1.1`, with two Logitech receivers beside it. Flipping it to another machine drops all three at once.

**Symptom:** flipped the switch away mid-song for 20s, then back. mpd logged `Failed to play on "NAD" (alsa): snd_pcm_writei() failed: No such device` but **stayed `[playing]`, with the clock running** (1:00 → 1:19, no output). On return, the udev `auto` saw the card's PCM `closed` and kicked: sound came back 3s after the card did, 23s further into the song. Left on the Steam box for an hour, the queue would play about 15 songs to nobody.

**Fix:** pause on unplug, resume on return. A udev `remove` rule runs `bars-output gone` through `systemd-run` (no delay). If mpd is playing and the enabled target's card is missing, it pauses and touches `/run/bars-output.paused`. `auto` checks for the marker first: it deletes the marker, and resumes if mpd is still paused. A pause jey made by hand leaves no marker, and if jey plays or stops while the card is away, the `paused` check fails and nothing resumes. `/run` clears at boot, so a cold boot never resumes. Auto-resume rather than stay-paused was jey's call.

**Deploy:**
```sh
scp scripts/bars-output config/90-bars-output.rules jukebox.local:/tmp/
ssh jukebox.local 'sudo install -m 755 /tmp/bars-output /usr/local/bin/
  sudo install -m 644 /tmp/90-bars-output.rules /etc/udev/rules.d/
  sudo udevadm control --reload && rm /tmp/bars-output /tmp/90-bars-output.rules'
```

**Verified:** the failing control was the flip above, on the old script. On the new one: the NAD dropped at 08:44:52, and `NAD left; paused` landed the same second. The NAD was back at 08:45:06, `card back; resumed` came at 08:45:08, the PCM was `RUNNING`, and no kick was needed (resuming from a pause reopens the card). jey heard it pick up at the same moment it stopped. The marker was gone afterwards.

**Side finding (open):** at boot, the udev coldplug `add` fires `auto` before mpd is listening. `MPD error: Connection refused` leaves a failed `run-*.service` on every boot with a card present. It's harmless, because the boot wait's `ExecStartPost` runs `auto` again once mpd is up, but it's red in the journal. The likely fix is that `auto` exits 0 when mpd isn't reachable.

---

## 2026-11-12 -- cava zeros: a 32-bit overflow after ~15 minutes of quiet

**Caught in the wild:** jey paused for about 30 minutes and pressed play. The music came back and the OLED stayed dark. The probe, run before restarting cava, showed `rchar +181080`/s on the live inodes (67, 68) and 0/32 nonzero. That is the **processing** hypothesis. A sample read straight off `/tmp/mpd.fifo` was real audio (4096/4096 samples nonzero, peak 25641). **The kernel log showed no USB disconnect in the 45 minutes before**, so the NAD never left the bus. `systemctl restart cava` fixed it.

**Not the cause: autosens.** cava 0.10.4's `cavacore.c` (the version on the box, `0.10.4+dfsg-1`) never touches `sens` on a silent frame (lines 515-526). It drops ×0.98 on overshoot and rises ×1.002 only when the frame isn't silent. So it can't drift during a pause.

**Root cause, from the source:** each frame where cava reads nothing, it bumps `frame_skip`. When samples come back, it updates its framerate estimate (line 362):

```c
p->framerate += (double)((p->rate * p->audio_channels * p->frame_skip) / new_samples) / 64;
```

`rate`, `audio_channels` and `frame_skip` are all `int` (`cavacore.h`), so the multiply runs in 32 bits *before* the cast. 44100 × 2 × `frame_skip` passes `INT_MAX` at `frame_skip` ≈ 24,350 and wraps negative. Then:

1. `framerate` goes negative, and `pow(60 / framerate, 2.5)` is NaN, so `gravity_mod` is NaN. `if (gravity_mod < 1)` is false for NaN, so the clamp doesn't catch it.
2. The falloff step turns each bar NaN, and the integral (`cava_mem[n] * noise_reduction + out`) keeps it NaN forever. Only a restart clears it.
3. On ARM64, converting NaN to an int gives 0. Hence exactly zero, with real audio going in.

cava keeps running about **26.6 frames/s while paused** (measured from `wchar`, 32 bytes a frame), so the wrap lands at **~15.3 minutes** of quiet. It wraps back positive at ~30.5 minutes, and negative again at ~45.8. Inferred, not observed: the NaN step itself (nothing reads cava's internal state from outside).

**Trials (same box, same NAD, same queue, only the hold length varied):**

| Hold | Frames | Predicted | +5s | +30s | USB disconnects |
|---|---|---|---|---|---|
| 10 min | ~16,000 | clean | 32/32 | 32/32 | 0 |
| 20 min | ~31,900 | dark | 0/32 | 0/32 | 0 |

**It explains the earlier data too.** All 9 cans trials (2026-11-11) held 5 minutes, which is never enough to reach the wrap. They weren't clean because of the cans. jey's "1 in 2 after a natural queue end (~30 min)" sits right on the 30.5-minute boundary. The 11-10 first sighting held ~25 minutes stopped, inside the dark band. "About 1 in 4 after 5+ minutes" was a mix of pause lengths on both sides of 15 minutes.

**Upstream:** master has reworked the line to `(double)(p->rate * p->frame_skip) / (new_samples / p->audio_channels)`. It's still an int multiply before the cast (master's field types not checked), which would put the wrap at ~30 minutes instead of ~15.

**Fix (provisional): `cava-guard`.** The guard is crust `scripts/cava-guard` plus `systemd/cava-guard.service`, running as root and `PartOf` mpd. It blocks on `mpc idleloop player`, notes when playback stops, and on the next play after ≥ 600s of quiet runs `systemctl restart cava`. It restarts at resume rather than during the quiet, because a restart mid-pause only resets the count and it starts climbing again. Audio-safe: mpd's FIFO output drains its own pipe, so cava going away for a second can't stall playback. jey's call over patching cava ourselves (we'd own a build) or feeding the FIFO during a pause (a second writer on mpd's pipe). **Delete it when a cava with the overflow fixed reaches Debian.** An upstream issue and PR are still to do.

**Deploy:**
```sh
scp scripts/cava-guard systemd/cava-guard.service jukebox.local:/tmp/
ssh jukebox.local 'sudo install -m 755 /tmp/cava-guard /usr/local/bin/
  sudo install -m 644 /tmp/cava-guard.service /etc/systemd/system/
  rm /tmp/cava-guard /tmp/cava-guard.service
  sudo systemctl daemon-reload && sudo systemctl enable --now cava-guard'
```

**Verified:** a wiring check first, run by hand with `CAVA_GUARD_QUIET_SECS=20`. A 25s pause restarted cava, and a 2s pause didn't. Then the control: the same 20-minute hold that went dark at 10:39 came back **32/32 at +5s and +30s** at 11:21, with `cava-guard: restarted cava after 1203s of quiet` in its journal. **The visible cost:** jey saw a few frames of flashing bars on the OLED at resume before they settled. That fits a fresh cava's autosens init (`sens` ×1.1 a frame until the first overshoot, lines 523-524), so expect it after every long pause the guard catches.

**Probe gap:** the jukebox has no `bc`. Use `awk` for arithmetic in anything run there.

---

## 2026-11-15 -- The shared layer stands alone

**Change:** crust's config now only carries what any one-Pi mpd → cava → crust stack needs. The jukebox's own pieces moved to bars, which installs them on top (see bars' integration log, same date).

- `mpd.conf` dropped the NAD and Cans outputs, `bind_to_address`, and `sticker_file`; they're in the box's `/etc/mpd_local.conf` now, which bars ships. The effective config is unchanged: the same 15 directives and blocks, only reordered.
- `10-audio-wait.conf` reads card ids from `mpd_local.conf` too (`abe58e9`), skips the wait when there are none (`77fde24`), and lost its `ExecStartPost=bars-output auto` (`96e245d`). **That line broke mpd on any box without bars:** a failed `ExecStartPost` marks the unit failed, and SETUP told standalone users to install it.
- `bars-output`, `90-bars-output.rules`, and `mout` left the repo (`23107ed`).

**Fix commands:** staged with `git archive` to `~/deploy/crust` (`4659f9d`) and `~/deploy/bars`, then installed per bars' `provisioning.md` Phases 5-7.

**Verified:**
- `systemctl show mpd -p DropInPaths` lists crust's `10-` and bars' `20-bars-output.conf`. With bars' drop-in moved aside, `ExecStartPost` comes back empty, so crust alone never calls `bars-output`. Restored.
- `systemctl restart mpd`: mpd, cava, crust, and cava-guard all came back under new PIDs. The `PartOf` cascade holds. Start took ~8s, with no failed units.
- Outputs renumbered (CAVA FIFO is output 1 now, since the include comes last). Nothing addresses them by number.
- jey pressed play on the deck: sound in the cans, bars on the OLED, PCM `RUNNING`, zero xruns.

## 2026-11-18 -- A key that can run one script

**Change:** `crust-control` installed behind a forced-command key for a new `control` user, per SETUP section 11 (`f26ac06`). A remote client (in bars' case the wallbox) asks for `status` over ssh and gets `key value` lines back; the key can run nothing else.

- `/usr/local/bin/crust-control`, root:root 755.
- `control` is a system user (uid 999), password locked, shell `/bin/bash` (Dropbear runs the forced command through it and wants it in `/etc/shells`).
- `/home/control`, `~/.ssh`, and `authorized_keys` are all root's (755, 755, 644), and the home has no dotfiles. **Why all three:** root-owning only the key file still let anything running as `control` move `~/.ssh` aside or write a `~/.bashrc` that bash reads for ssh sessions. Either gets around `restrict,command=`. Dropbear accepts root ownership as long as nothing is group- or world-writable (`checkfileperm`, svr-authpubkey.c).
- `/etc/crust/status.d/`, root:root 755, empty.
- The client's key was made on the client; only its `.pub` line came to the Pi.

**Red herrings:** `passwd -l` looked like it would lock the key out too, since Dropbear has a "User account is locked" message. It's in the password-auth path only (svr-authpasswd.c); pubkey auth never reads the hash. And `BASH_ENV` looked like a way in before the script's first line, but Dropbear 2025.89 doesn't handle `env` channel requests at all, so a client can't set anything.

**Fix commands** (on the Pi, after `scp scripts/crust-control <pi>:`):
```
sudo install -o root -g root -m 755 ~/crust-control /usr/local/bin/crust-control && rm ~/crust-control
sudo useradd --system --no-create-home --home-dir /home/control --shell /bin/bash control
sudo passwd -l control
sudo install -d -o root -g root -m 755 /home/control /home/control/.ssh /etc/crust/status.d
echo 'restrict,command="/usr/local/bin/crust-control" ssh-ed25519 AAAA… client' | sudo tee /home/control/.ssh/authorized_keys >/dev/null
sudo chown root:root /home/control/.ssh/authorized_keys && sudo chmod 644 /home/control/.ssh/authorized_keys
```

**Verified,** from the client with `-o IdentitiesOnly=yes -i <key>`:
- `status`: all ten lines, exit 0.
- Refused with `err usage: status`, exit 2: `id`, `status; touch /tmp/cc-pwned`, `$(touch /tmp/cc-pwned)`, a bare shell request, and `echo x > .bashrc`. No marker file.
- Refused by `restrict`: a forced pty (`-tt`; the client gives up with 255), sftp (the forced command answers in its place), scp in both protocols, `-W` to mpd's port, and a `-R` forward.
- A probe drop-in, run as `control` inside a real session with `-A` and a loaded agent: no `SSH_AUTH_SOCK`, no tty, and it couldn't write the home, move `~/.ssh`, or append to the key file. Its control write to `/var/tmp` succeeded, so the `no`s mean something. The same writes via `sudo -u control` were refused by the filesystem. The probe and its marker were removed.
- mpd idle throughout; `status` takes ~0.17s.
