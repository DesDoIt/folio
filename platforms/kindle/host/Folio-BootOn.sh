#!/bin/sh
# Name: Folio OS: boot ON
# Author: folio
# Makes the Kindle show the Folio OS boot menu at power-on (5 s countdown, defaults to Folio OS).
FB=/mnt/us/libkh/bin/fbink
say() { [ -x "$FB" ] && "$FB" -q -y -6 -m "$*" >/dev/null 2>&1; }
if [ ! -f /mnt/us/kos/folio.conf ] || [ ! -f /mnt/us/kos/kos-boot.sh ]; then
    say "Folio OS boot files missing in kos/"; exit 1
fi
# install the hook first, then drop the one from before the rename (Kindle Linux), so the
# boot menu never runs twice
mntroot rw && cp /mnt/us/kos/folio.conf /etc/upstart/folio.conf && rm -f /etc/upstart/kindle-linux.conf; rc=$?
mntroot ro
rm -f /mnt/us/kos/boot-pending
if [ $rc -eq 0 ]; then say "Boot into Folio OS: ON"; else say "Failed to enable boot (see kos.log)"; fi
