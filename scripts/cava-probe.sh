#!/bin/bash

# Snapshot cava's state, read-only. Run it when the OLED goes dark during
# playback, BEFORE restarting cava -- the restart destroys the evidence.
#
# Tells apart the "cava writes zeros" hypotheses (crust debug-log, 2026-11-11):
#   input stuck   -- rchar flat: cava stopped reading /tmp/mpd.fifo
#   stale pipe    -- cava's read fd points at an inode other than /tmp/mpd.fifo
#   processing    -- rchar climbing ~176KB/s on the live inode, output still zero
#
# Reads 64 bytes of /tmp/cava.fifo, which crust also reads: crust loses about
# one frame. Never reads /tmp/mpd.fifo -- that would steal cava's input.

pid=$(pidof cava) || { echo "cava not running"; exit 1; }

echo "== $(date -Is)"
mpc status | sed -n '2p'
echo "cava pid $pid, up $(ps -o etime= -p "$pid" | tr -d ' ')"

echo "-- io over 1s (bytes)"
read_io() { awk '/^rchar|^wchar/ {printf "%s ", $2}' "/proc/$pid/io"; }
before=($(read_io)); sleep 1; after=($(read_io))
echo "rchar +$((after[0] - before[0]))  wchar +$((after[1] - before[1]))"

echo "-- fifo inodes"
echo "mpd.fifo  on disk: $(stat -c %i /tmp/mpd.fifo)"
echo "cava.fifo on disk: $(stat -c %i /tmp/cava.fifo)"
for fd in /proc/"$pid"/fd/*; do
  target=$(readlink "$fd")
  case "$target" in *fifo*|pipe:*) echo "fd ${fd##*/} -> $target (inode $(stat -L -c %i "$fd"))" ;; esac
done

echo "-- threads (state, wchan)"
for t in /proc/"$pid"/task/*; do
  echo "${t##*/} $(awk '{print $3}' "$t/stat") $(cat "$t/wchan")"
done

echo "-- cava.fifo sample"
sample=$(timeout 1 head -c 64 /tmp/cava.fifo | od -v -An -tu2 | tr -s ' ')
echo "$sample"
nonzero=$(echo "$sample" | tr ' ' '\n' | grep -cv '^0*$')
echo "nonzero values: $nonzero / 32"
