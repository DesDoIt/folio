#!/bin/sh
# Runs inside the Alpine chroot. Started by host/kos-launch.sh.
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export HOME=/root LANG=C.UTF-8
mkdir -p /run /var/lib/kos /tmp
exec /opt/kos/kos
