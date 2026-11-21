#!/bin/sh
# Name: Kindle Linux: boot ON
# Author: kindle-linux
# Makes the Kindle show the Kindle Linux boot menu at power-on (5 s countdown, defaults to Kindle Linux).
FB=/mnt/us/libkh/bin/fbink
say() { [ -x "$FB" ] && "$FB" -q -y -6 -m "$*" >/dev/null 2>&1; }
if [ ! -f /mnt/us/kos/kindle-linux.conf ] || [ ! -f /mnt/us/kos/kos-boot.sh ]; then
    say "Kindle Linux boot files missing in kos/"; exit 1
fi
mntroot rw && cp /mnt/us/kos/kindle-linux.conf /etc/upstart/kindle-linux.conf; rc=$?
mntroot ro
rm -f /mnt/us/kos/boot-pending
if [ $rc -eq 0 ]; then say "Boot into Kindle Linux: ON"; else say "Failed to enable boot (see kos.log)"; fi
