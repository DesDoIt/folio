#!/bin/sh
# Kindle Linux boot hook - started once per boot by /etc/upstart/kindle-linux.conf after the stock
# UI (lab126_gui) has started. Shows a boot menu, then starts Kindle Linux or leaves the stock Kindle.
#
# Safety:
#   * kos/disable on the USB drive        -> always stock Kindle
#   * kos/boot-pending left over          -> the previous Kindle Linux boot never drew its first
#                                            screen (hang + forced reboot): boot stock this time
#   * runs at most once per boot (/var/run/kos.booted), so exiting to the stock Kindle later
#     (which restarts lab126_gui) doesn't loop back into Kindle Linux

KOS=/mnt/us/kos
LOG=$KOS/kos.log
log() { echo "$(date '+%Y-%m-%d %H:%M:%S') [boot] $*" >> "$LOG"; }

[ -e /var/run/kos.booted ] && exit 0
touch /var/run/kos.booted

# wait for the userstore (it's mounted by the time lab126_gui runs, but be defensive)
i=0; while [ ! -d "$KOS" ] && [ $i -lt 30 ]; do sleep 1; i=$((i + 1)); done
[ -d "$KOS" ] || exit 0

if [ -f "$KOS/disable" ]; then log "kos/disable present: staying on stock Kindle"; exit 0; fi
if [ -f "$KOS/boot-pending" ]; then
    rm -f "$KOS/boot-pending"
    log "previous Kindle Linux boot did not finish: staying on stock Kindle this time"
    exit 0
fi
[ -x "$KOS/kos" ] && { [ -f "$KOS/kos-rootfs.sqsh" ] || [ -f "$KOS/kos-rootfs.ext3" ]; } || { log "kos binary or image missing"; exit 0; }

# Give the stock UI a moment to settle, then take the screen for the boot menu.
sleep 3
stop lab126_gui >/dev/null 2>&1
sleep 1
"$KOS/kos" --boot-menu 5
choice=$?
if [ "$choice" -ne 0 ]; then
    log "boot menu: stock Kindle chosen"
    start lab126_gui >/dev/null 2>&1
    exit 0
fi

log "boot menu: Kindle Linux"
touch "$KOS/boot-pending"     # removed by the GUI after its first frame
KOS_BOOT=1 setsid sh "$KOS/kos-launch.sh" </dev/null >/dev/null 2>&1 &
exit 0
