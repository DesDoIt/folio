#!/bin/sh
# Name: Kindle Linux: boot OFF
# Author: kindle-linux
# The Kindle boots straight into the stock interface again. Kindle Linux stays available from the library.
FB=/mnt/us/libkh/bin/fbink
say() { [ -x "$FB" ] && "$FB" -q -y -6 -m "$*" >/dev/null 2>&1; }
mntroot rw && rm -f /etc/upstart/kindle-linux.conf; rc=$?
mntroot ro
if [ $rc -eq 0 ]; then say "Boot into Kindle Linux: OFF"; else say "Failed to disable boot"; fi
