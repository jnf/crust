# Plan: block the render loop on new data

**Branch:** `block-render-on-new-data` (off `canon`)
**Status:** Phase 1 done & verified on device (`cab8468`). Phase 2 (dirty check) pending.

## Results (Phase 1, measured on `bars.local`, same track playing)

| Metric | Before (free-run) | After (block-on-data) |
|---|---|---|
| I2C interrupts/sec | ~6,989 | ~2,800 (−60%) |
| Total interrupts/sec | ~9,100 | ~3,800 |
| crust time in `D` (I2C xfer) | ~94% | ~30% |
| crust CPU% | 8.3% | 3.6% |
| mpd "decoder too slow" | every 3–5 min | none in 5+ min |

The iowait aggregate (`vmstat` `wa`) stayed ~24% and looked unchanged — that's a known multicore-Linux artifact (it attributes a blocked task's wait to whatever core is idle). The honest signal is the per-task `D`/`S` sampling above, which confirms crust's I2C blocking dropped from continuous to ~30%.

---

## The problem

crust is starving mpd. Profiling `bars.local` while it played showed:

- mpd logging `alsa_output: Decoder is too slow; playing silence to avoid xrun` every few minutes — the audible stutter. Not a buffer xrun; a **scheduling-starvation** symptom.
- The I2C controller is the dominant interrupt source on the whole system: **~7,100 interrupts/sec**, ~80% of all interrupts.
- crust's render thread sits in `D` (uninterruptible sleep) inside `bcm2835_i2c_xfer` ~22% of the time.

Root cause is in [src/main.rs:47-69](src/main.rs#L47-L69). The reader thread drains `cava.fifo` continuously (good — that's the anti-xrun design), but the **render loop is decoupled from cava's cadence and has no throttle**:

```rust
loop {
    let buf = *shared.lock().unwrap();   // whatever the reader last wrote — new or not
    // clear, draw bars
    display.flush()...                   // ~15ms blocking I2C blast
}                                         // immediately loop again
```

cava emits at ~30fps. The render loop re-draws and re-flushes as fast as `flush()` returns (~60–85fps), **re-pushing the same frame over I2C whenever cava hasn't produced anything new**. During quiet passages the bars are identical frame to frame and it's *still* blasting the full framebuffer. The bus does 2–3× the work cava justifies, and the resulting interrupt storm periodically preempts mpd's decoder thread — which runs as plain SCHED_OTHER with no priority to defend itself.

---

## The fix: render only when there's new data

Tie the render rate to the *data* rate instead of the bus speed. The render loop should **block until the reader has a fresh frame**, render it, then block again. With cava at 30fps, render drops to ~30fps and I2C traffic roughly halves immediately.

### The fork (decided)

| Option | Shape | Verdict |
|---|---|---|
| **`mpsc` channel** | Reader *sends* each frame; render `recv()` blocks until one arrives. | **Chosen.** Deletes the `Mutex` entirely; producer/consumer is exactly what channels model. |
| `Mutex` + `Condvar` + dirty flag | Keep the shared buffer; reader notifies a condvar on write; render waits on it. | Rejected — more moving parts (lock + condvar + flag, spurious-wakeup loop) for no benefit here. |

### What `mpsc` is (the learning bit)

`std::sync::mpsc` = **m**ulti-**p**roducer, **s**ingle-**c**onsumer. It's a thread-safe queue split into two ends:

- `let (tx, rx) = mpsc::channel();` gives a **sender** (`tx`) and a **receiver** (`rx`).
- The sender's `tx.send(value)` hands a value to the queue. Ownership *moves* through the channel — the value is gone from the sender's side, no shared memory, no lock to hold.
- The receiver's `rx.recv()` **blocks the calling thread until a value is available**, then returns it. That blocking *is* our "wait for new data" — it replaces the busy free-run with the thread parked, consuming zero CPU until cava produces.

"Multi-producer" means you can clone `tx` and have many senders; "single-consumer" means there's exactly one `rx`. Our case is the simplest one: one reader thread sending, one render loop receiving.

Two channel flavors matter here:

- `mpsc::channel()` — **unbounded**. `send()` never blocks.
- `mpsc::sync_channel(n)` — **bounded** to `n` slots. `send()` blocks when full.

**We want unbounded.** The reader thread must keep draining `cava.fifo` no matter what — if it ever blocks, the cava→mpd pipeline backs up and we're right back to xruns (the whole reason the reader thread exists). An unbounded channel guarantees the sender never blocks. We handle staleness on the *receiver* side instead (below).

### Design

```rust
use std::sync::mpsc;

let (tx, rx) = mpsc::channel::<[u8; FRAME_BYTES]>();

// Reader thread: drain the FIFO, hand each frame to the channel. Never blocks.
thread::spawn(move || {
    let mut fifo = File::open("/tmp/cava.fifo").expect("Failed to open /tmp/cava.fifo");
    let mut buf = [0u8; FRAME_BYTES];
    loop {
        fifo.read_exact(&mut buf).expect("FIFO read error");
        if tx.send(buf).is_err() {
            break; // render side hung up; nothing left to do
        }
    }
});

// Render loop: block for a frame, then drain to the freshest one before drawing.
loop {
    let mut frame = match rx.recv() {
        Ok(f) => f,
        Err(_) => break, // reader thread gone — exit, let systemd restart us
    };
    while let Ok(f) = rx.try_recv() {
        frame = f; // skip stale frames; only the newest matters
    }

    // draw `frame` exactly as today, then flush
}
```

**Why drain-to-latest** (`recv()` then a `try_recv()` loop): if the render flush ever falls behind cava for a burst (an I2C stall, say), frames pile up in the unbounded queue. We don't want to render them in sequence and lag real-time — we want the *newest*. `recv()` blocks for the first frame; `try_recv()` (non-blocking) pulls any extras without waiting; we keep only the last. This guarantees render rate ≤ cava rate and the display never drifts behind the audio. In steady state the render flush (~15ms) is faster than cava's cadence (~33ms), so the queue normally holds 0–1 frames and the drain is a no-op — but it's cheap insurance.

### Side benefit: cleaner failure mode

Today, if the reader thread panics (e.g. cava dies and the FIFO read errors), the render loop spins forever on the last stale frame. With the channel, a dead reader drops `tx`, `rx.recv()` returns `Err`, render breaks, `main` returns, the process exits — and `Restart=on-failure` in [the crust unit](systemd/) brings it back cleanly. Strictly better than today.

---

## Phases

**Phase 1 — the core change (one hard thing).** Replace the `Mutex` + busy-loop with the `mpsc` channel + drain-to-latest. Keep the draw/flush logic byte-for-byte identical. Cross-build, deploy, verify on device, commit.

**Phase 2 — probe the hard variant (follow-up).** Add a dirty check: skip `flush()` when the rendered framebuffer is unchanged from the last flush. Kills I2C traffic to ~zero during silence/static. Measure the additional interrupt drop; if the extra state isn't worth it, retreat with data. Separate commit.

Keep Phase 1 thin and settled before touching Phase 2.

---

## Verification (on `bars.local`)

Baseline (already captured): ~7,100 I2C int/sec, ~22% iowait, "decoder too slow" every few minutes.

After Phase 1, with audio playing:

1. **I2C interrupt rate** — should roughly halve, to ~cava's frame rate:
   ```sh
   a=$(grep 3f804000.i2c /proc/interrupts | awk '{s=0;for(i=2;i<=5;i++)s+=$i;print s}'); \
   sleep 1; \
   b=$(grep 3f804000.i2c /proc/interrupts | awk '{s=0;for(i=2;i<=5;i++)s+=$i;print s}'); \
   echo "I2C int/sec: $((b-a))"
   ```
2. **The mpd symptom is gone** — watch for silence insertions:
   ```sh
   journalctl -u mpd -f | grep -i "too slow"
   ```
   Should stop firing.
3. **iowait drops** — `vmstat 1 5`, the `wa` column and the blocked-task count `b`.
4. **Visuals still smooth** — bars react to audio at ~30fps; no visible lag or stutter on the OLED.

Expected: render ~66fps → ~30fps, I2C int/sec ~7,100 → ~3,500, "too slow" gone. Phase 2 then takes quiet-passage traffic toward zero.

---

## Open questions / corners

- **Is ~30fps enough visually?** Almost certainly yes for a 16-bar spectrum, but confirm on the device — if it feels choppy we revisit (cava framerate is configurable).
- **mpd priority** — out of scope for this branch. Even after this fix lands, giving mpd RT scheduling (or negative nice) is worthwhile belt-and-suspenders. Tracking it separately so it doesn't ride inside this feature.
