#!/bin/sh
# Name: Folio OS
# Author: folio
# DontUseFBInk
# Starts Folio OS detached, so it survives the Kindle UI being stopped.
if command -v setsid >/dev/null 2>&1; then
    setsid sh /mnt/us/kos/kos-launch.sh </dev/null >/dev/null 2>&1 &
else
    nohup sh /mnt/us/kos/kos-launch.sh </dev/null >/dev/null 2>&1 &
fi
