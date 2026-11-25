#!/bin/sh
# Folio OS boot hook - started once per boot by /etc/upstart/folio.conf (kindle-linux.conf before the
# rename to Folio OS) after the stock UI (lab126_gui) has started. Shows a boot menu, then starts
# Folio OS or leaves the stock Kindle.
#
# Safety:
#   * kos/disable on the USB drive        -> always stock Kindle
#   * kos/boot-pending left over          -> the previous Folio OS boot never drew its first
#                                            screen (hang + forced reboot): boot stock this time
#   * runs at most once per boot (/var/run/kos.booted), so exiting to the stock Kindle later
#     (which restarts lab126_gui) doesn't loop back into Folio OS

KOS=/mnt/us/kos
LOG=$KOS/kos.log
log() { echo "$(date '+%Y-%m-%d %H:%M:%S') [boot] $*" >> "$LOG"; }

# mkdir is atomic: if both the old and the new hook are installed, only one of them gets past here
mkdir /var/run/kos.booted 2>/dev/null || exit 0

# wait for the userstore (it's mounted by the time lab126_gui runs, but be defensive)
i=0; while [ ! -d "$KOS" ] && [ $i -lt 30 ]; do sleep 1; i=$((i + 1)); done
[ -d "$KOS" ] || exit 0

# Installs from before the rename to Folio OS have the hook as /etc/upstart/kindle-linux.conf. Once the
# update has put folio.conf in kos/, switch to it (what "Folio OS: boot ON" does) so "Folio OS: boot OFF"
# finds it. Any failure leaves the old hook in place, which keeps working.
migrate_hook() {
    [ -f /etc/upstart/kindle-linux.conf ] && [ -f "$KOS/folio.conf" ] || return 0
    command -v mntroot >/dev/null 2>&1 || return 0
    mntroot rw >/dev/null 2>&1 || { log "hook migration: can't remount / read-write"; return 0; }
    if [ -f /etc/upstart/folio.conf ] || cp "$KOS/folio.conf" /etc/upstart/folio.conf; then
        rm -f /etc/upstart/kindle-linux.conf && log "boot hook: replaced kindle-linux.conf with folio.conf"
    else
        rm -f /etc/upstart/folio.conf
        log "hook migration: copying folio.conf failed; keeping kindle-linux.conf"
    fi
    mntroot ro >/dev/null 2>&1 || log "hook migration: can't remount / read-only"
}
migrate_hook

if [ -f "$KOS/disable" ]; then log "kos/disable present: staying on stock Kindle"; exit 0; fi
if [ -f "$KOS/boot-pending" ]; then
    rm -f "$KOS/boot-pending"
    log "previous Folio OS boot did not finish: staying on stock Kindle this time"
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

log "boot menu: Folio OS"
touch "$KOS/boot-pending"     # removed by the GUI after its first frame
KOS_BOOT=1 setsid sh "$KOS/kos-launch.sh" </dev/null >/dev/null 2>&1 &
exit 0
