#!/bin/sh
# Name: Folio OS: boot OFF
# Author: folio
# The Kindle boots straight into the stock interface again. Folio OS stays available from the library.
FB=/mnt/us/libkh/bin/fbink
say() { [ -x "$FB" ] && "$FB" -q -y -6 -m "$*" >/dev/null 2>&1; }
# kindle-linux.conf: the hook's name before the rename to Folio OS
mntroot rw && rm -f /etc/upstart/folio.conf /etc/upstart/kindle-linux.conf; rc=$?
mntroot ro
if [ $rc -eq 0 ]; then say "Boot into Folio OS: OFF"; else say "Failed to disable boot"; fi
