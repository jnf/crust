# crust — working notes for Claude

Real-time audio spectrum visualizer on a Raspberry Pi Zero 2W: CAVA's FFT → FIFO → this Rust binary → SSD1305 OLED over I2C. Resource-constrained embedded target — keep that in mind for every change.

This is the shared layer, kept free of any one machine's details. **Instance specifics — your Pi's hostname, deploy target, local quirks — go in `CLAUDE.local.md` (gitignored, never committed).** See the bottom for what belongs there.

## Deploy

crust is cross-compiled on macOS for aarch64 — never built on the Pi:

```sh
cargo zigbuild --release # target pinned in .cargo/config.toml
scp target/aarch64-unknown-linux-gnu/release/crust <user>@<pi>:~/crust
ssh <user>@<pi> 'sudo systemctl restart crust'
```

Be your own CI: `cargo zigbuild --release` has to pass before anything ships.

## Log every Pi-side change in debug-log.md

The core convention. `debug-log.md` is the canonical history of everything done to the Pi *outside* the binary — config, systemd, hardware, diagnoses. Every such change earns an entry: problem, root cause, the exact fix commands, and how it was verified. Match the existing voice: terse, root-cause-first, honest about dead ends and red herrings.

## Where things live

- `src/` — the binary (`main.rs`, `display.rs`). Small on purpose.
- `systemd/` — service units + drop-ins, version-controlled, deployed to `/etc/systemd/system/`. They carry concrete user/paths; edit them for your user (see SETUP.md).
- Pi-side config: `/etc/mpd.conf` and `~/.config/cava/config`, documented in SETUP.md.
- `SETUP.md` — first-time setup. `README.md` — architecture + bar-render math. `plans/` — design docs for bigger changes.

## Invariants — don't regress these (each cost a debug-log entry to find)

- **Render blocks on new data.** The render loop waits on an mpsc channel `recv()`, drains to the latest frame, and never free-runs. A free-running flush floods I2C with interrupts and starves MPD's decoder → audio dropouts. (2026-06-10)
- **Dirty-check the flush.** Skip the I2C write when bar heights are unchanged; silence → zero I2C traffic.
- **FIFO re-sync is automatic.** cava/crust are `PartOf` MPD's restart cascade. Don't sever it — restarting MPD recreates `/tmp/mpd.fifo`, and without the cascade CAVA emits silence to a blank OLED. (2026-06-11)
- **`plughw`, not `hw`.** ALSA outputs need the plugin layer to convert our 44100/16-bit PCM to the device's native format (S32_LE on a USB DAC, IEC958 on HDMI).
- **`bars = 16` in CAVA.** It writes 32 bytes/frame regardless; other values cause frame-boundary drift.

## Audio output

Playback goes to an ALSA output defined in `mpd.conf`; the visualizer FIFO branch is independent of the playback device, so the OLED works regardless of where audio is routed. See SETUP.md for the signal chain and how to switch outputs.

## Instance layer — `CLAUDE.local.md`

Anything true of *one* deployment rather than the project lives there (gitignored): the Pi's hostname and your SSH/deploy target, whether sudo is passwordless, the specific audio hardware, any standing "OK to do X on my box" permissions. Create your own; don't commit it.
