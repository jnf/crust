#!/bin/bash

# Turn off the OLED display and stop crust.
# Control byte 0x00 = command stream; 0xAE = display off
# Full path: i2c-tools installs to /usr/sbin, which isn't on a non-root PATH
# on Debian (debug-log, 2026-11-11). jey's i2c group membership covers access.

mpc stop
sudo systemctl stop cava crust
/usr/sbin/i2ctransfer -y 1 w2@0x3c 0x00 0xae
