#!/bin/bash

# Turn off the OLED display and stop crust.
# Control byte 0x00 = command stream; 0xAE = display off

mpc stop
sudo systemctl stop cava crust
i2ctransfer -y 1 w2@0x3c 0x00 0xae
