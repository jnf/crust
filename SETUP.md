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
sudo apt install -y mpd mpc cava i2c-tools alsa-utils
```

`alsa-utils` brings `aplay`, which the boot-time audio wait (step 6) uses to probe your card. Some images ship it; DietPi doesn't.

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

**`10-audio-wait.conf`** — MPD opening an ALSA output before the device is ready gets "Unknown error 524" and silently fails. This `ExecStartPre` loops a 1-second silent `aplay` probe across every `CARD=` id in `mpd.conf` and `mpd_local.conf` until one accepts, then unblocks MPD. No card isn't a failure: after 60s it gives up and starts MPD anyway, since a DAC behind a USB switch is often absent at boot. Outputs addressed without `CARD=` (`plughw:0,0`, `default`) have nothing to probe, so MPD starts at once. Needs `alsa-utils` for `aplay`.

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

**cava-guard** works around a cava 0.10.4 bug: after about 15 minutes paused, a 32-bit overflow makes cava write zeros, so the music plays but the OLED stays dark. It restarts cava on the first play after 10+ minutes of quiet (debug-log 2026-11-12). Drop it once your distro ships a fixed cava.

```sh
sudo install -m 755 scripts/cava-guard /usr/local/bin/
sudo cp systemd/cava-guard.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now cava-guard
```

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

**Stop everything and blank the display** (stops playback, cava and crust, then turns the OLED off):

```sh
scripts/stop.sh
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

## 11. Control plane

A remote client asks the Pi for its health over ssh. The key it uses can run `scripts/crust-control` and nothing else: no shell, no other command, no forwarding.

`status` prints one fact per line, `key value…`, units in the key names:

```text
v 1
at 2026-11-17T10:42:05-08:00
up_s 183402
temp_c 51.2
mem_avail_mb 214
disk_free_gb 402
svc mpd active
svc cava active
svc crust failed
svc cava-guard active
```

A field whose source can't be read is left out, not guessed. Anything but `status` answers `err usage: status` and exits 2.

**1. Install the script**, root-owned, where the key will point:

```sh
sudo install -o root -g root -m 755 scripts/crust-control /usr/local/bin/crust-control
```

**2. Create the `control` user.** Its own user, so the key never lands on an account with sudo. The shell has to be a real one: sshd runs the forced command through it.

```sh
sudo useradd --system --create-home --shell /bin/bash control
sudo passwd -l control
```

**3. Authorize the client's key** with a forced command. `restrict` turns off ptys, forwarding and the rest; `command=` replaces whatever the client asks for with the script:

```sh
sudo install -d -o root -g root -m 755 ~control/.ssh
echo 'restrict,command="/usr/local/bin/crust-control" ssh-ed25519 AAAA… client' \
  | sudo tee ~control/.ssh/authorized_keys >/dev/null
sudo chown root:root ~control/.ssh/authorized_keys
sudo chmod 644 ~control/.ssh/authorized_keys
```

The key file and its directory are root's, so anything that ever runs as `control` can't rewrite its own key line and drop the `restrict,command=` in front of it. The home directory is still `control`'s, as `useradd` made it; the key file deliberately isn't.

Needs an ssh server that honors `restrict` and `command=` in `authorized_keys`: OpenSSH does, and so does Dropbear (checked on v2025.89).

> **The client's command is untrusted input.** It reaches the script as `$SSH_ORIGINAL_COMMAND`, raw, `;` and `$(…)` included, so the script only ever compares it to its fixed verb list; never `eval` it or pass it on.

**4. Make the drop-in directory**, root-owned so only root can add to what `status` runs:

```sh
sudo install -d -o root -g root -m 755 /etc/crust/status.d
```

Empty is normal. Each regular, executable file in it runs after the script's own lines, in name order, as the `control` user, with no arguments, no stdin and 5 seconds. Its `key value` lines are added to `status`: keys are lowercase (`[a-z][a-z0-9_]*`), `v` and `at` are the script's own, and anything else is dropped, as is stderr. A drop-in that fails or hangs loses only its own lines. Drop-ins add lines; they never add verbs.

**5. Check it** from the client:

```sh
ssh -i <key> control@<pi> status   # the lines above, exit 0
ssh -i <key> control@<pi> id       # err usage: status, exit 2 -- not a uid
```

The script's own tests run anywhere with bash 4.3+ (they stub `/proc`, `systemctl` and friends): `bash scripts/crust-control-test`.

---

For technical details on the visualization pipeline and bar layout, see [README.md](README.md).
