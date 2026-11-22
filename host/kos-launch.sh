#!/bin/sh
# Kindle Linux launcher - runs on the Kindle (host side, busybox ash).
# Mounts the Alpine image, stops Amazon's UI and services, runs the GUI in a chroot,
# then ALWAYS tears everything down and restores the stock Kindle.
#
# Escape hatches:
#   * create /mnt/us/kos/disable to stop kindle-os from starting at all
#   * hold the power button ~15 s to force a reboot (stock Kindle comes back)

KOS=/mnt/us/kos
# compressed read-only image; an older ext3 image still works
IMG=$KOS/kos-rootfs.sqsh
[ -f "$IMG" ] || IMG=$KOS/kos-rootfs.ext3
R=/var/tmp/kos-root
LOG=$KOS/kos.log
LOCK=/var/run/kos.lock
STOPPED=""
# Amazon reader/indexer/updater services. They don't stop with lab126_gui.
# Left running they eat CPU and RAM (256 MB total), so we keep them stopped while we run.
AMAZON_EXTRA="stored webreader kfxreader kfxview todo tmd rcm archive scanner otav3 otaupd"

log() { echo "$(date '+%Y-%m-%d %H:%M:%S') [launch] $*" >> "$LOG"; }
msg() { [ -x /mnt/us/libkh/bin/fbink ] && /mnt/us/libkh/bin/fbink -q -y -6 -m "$*" >/dev/null 2>&1; }

[ -f "$KOS/disable" ] && { msg "Kindle Linux disabled ($KOS/disable)"; exit 0; }
[ -f "$IMG" ] || { msg "Kindle Linux: $IMG missing"; exit 1; }
mkdir "$LOCK" 2>/dev/null || { log "already running"; exit 0; }

# keep the log from growing forever
[ -f "$LOG" ] && [ "$(wc -c < "$LOG")" -gt 1000000 ] && mv "$LOG" "$LOG.old"
log "starting Kindle Linux"

stop_job() {
    if status "$1" 2>/dev/null | grep -q "start/"; then
        stop "$1" >/dev/null 2>&1 && STOPPED="$1 $STOPPED" && log "stopped $1"
    fi
}

LOOPFILE=/var/run/kos.loop

# Undo anything a previous session left behind (a lazy unmount can keep a loop device busy,
# and the Kindle only has a few of them: Amazon already uses 9).
release_stale() {
    if grep -q " $R" /proc/mounts; then
        log "cleaning up mounts from a previous session"
        umount_all
    fi
    for l in $(losetup -a 2>/dev/null | grep "kos-rootfs" | cut -d: -f1); do
        losetup -d "$l" 2>/dev/null && log "released stale loop device $l"
    done
}

# First unused loop device. The Kindle uses devfs-style names (/dev/loop/N), which busybox's
# `losetup -f` gets wrong (it answers /dev/loopN), so check the nodes that really exist.
find_loop() {
    for d in /dev/loop/* /dev/loop[0-9]*; do
        [ -b "$d" ] || continue
        losetup "$d" >/dev/null 2>&1 && continue   # already bound to a file
        echo "$d"
        return 0
    done
    return 1
}

mount_all() {
    mkdir -p "$R" || return 1
    release_stale
    LOOP=$(find_loop)
    if [ -z "$LOOP" ]; then
        log "no free loop device; devices: $(ls /dev/loop* /dev/loop/ 2>/dev/null | tr '\n' ' '); in use: $(losetup -a 2>/dev/null | tr '\n' ';')"
        return 1
    fi
    if ! losetup "$LOOP" "$IMG" 2>>"$LOG"; then
        log "losetup $LOOP failed"
        return 1
    fi
    echo "$LOOP" > "$LOOPFILE"
    case "$IMG" in
        *.sqsh) FSTYPE=squashfs; OPTS=ro ;;
        *) FSTYPE=ext3; OPTS=noatime
           command -v e2fsck >/dev/null 2>&1 && e2fsck -p "$LOOP" >> "$LOG" 2>&1 ;;
    esac
    if ! mount -t "$FSTYPE" -o "$OPTS" "$LOOP" "$R" 2>>"$LOG"; then
        log "mount image failed on $LOOP; kernel says: $(dmesg | tail -4 | tr '\n' ' ')"
        losetup -d "$LOOP" 2>/dev/null
        rm -f "$LOOPFILE"
        return 1
    fi
    log "image ($FSTYPE) mounted via $LOOP"
    for d in proc sys dev dev/pts mnt/us kindle; do mkdir -p "$R/$d" 2>/dev/null; done
    mount -t proc proc "$R/proc"
    mount -t sysfs sys "$R/sys"
    mount -o bind /dev "$R/dev"
    mount -o bind /dev/pts "$R/dev/pts"
    mount -o bind /mnt/us "$R/mnt/us"
    # Kindle root, read-only, for lipc/iptables helpers (chroot /kindle ...)
    mount -o bind / "$R/kindle" && mount -o remount,ro,bind "$R/kindle" 2>/dev/null
    for d in var dev proc sys; do mount -o bind "/$d" "$R/kindle/$d" 2>/dev/null; done
    mkdir -p "$R/run" "$R/tmp" 2>/dev/null
    mount -t tmpfs -o size=16m tmpfs "$R/run"
    mount -t tmpfs -o size=32m tmpfs "$R/tmp"
    if [ "$FSTYPE" = squashfs ]; then
        # the image is read-only: give /etc a writable copy in RAM (DNS, password, time zone)
        ETC=/tmp/kos-etc.$$
        rm -rf "$ETC"; cp -a "$R/etc" "$ETC" && mount -t tmpfs -o size=8m,mode=0755 tmpfs "$R/etc" \
            && cp -a "$ETC/." "$R/etc/" && log "etc: writable copy in RAM"
        rm -rf "$ETC"
    fi
    keep_state
    return 0
}

# Settings, saved Wi-Fi networks, the SSH host key and root password live on the Kindle's own
# persistent partition (not the USB drive, so passwords aren't exposed over USB), so they survive
# reinstalling or updating the image.
PERSIST=/var/local/kindle-linux
keep_state() {
    mkdir -p "$PERSIST/dropbear" "$PERSIST/root" || return
    mkdir -p "$R/var/lib/kos" "$R/etc/dropbear" "$R/root" 2>/dev/null
    if [ ! -f "$PERSIST/.created" ]; then
        cp -a "$R/var/lib/kos/." "$PERSIST/" 2>/dev/null   # first run: keep what this image already has
        touch "$PERSIST/.created"
    fi
    mount -o bind "$PERSIST" "$R/var/lib/kos" && log "state: $PERSIST"
    mount -o bind "$PERSIST/dropbear" "$R/etc/dropbear"
    mount -o bind "$PERSIST/root" "$R/root"   # root's home: shell history, editor settings
    if [ -s "$PERSIST/root-shadow" ]; then
        line=$(head -1 "$PERSIST/root-shadow")
        case "$line" in root:*) sed -i "s|^root:.*|$line|" "$R/etc/shadow" ;; esac
    fi
}

# Kill chroot processes with a given command name (e.g. our wpa_supplicant), leaving the rest.
kill_chroot_named() {
    for p in /proc/[0-9]*; do
        root=$(readlink "$p/root" 2>/dev/null)
        case "$root" in "$R"|"$R"/*) ;; *) continue ;; esac
        name=$(cat "$p/comm" 2>/dev/null)
        for n in "$@"; do [ "$name" = "$n" ] && kill "${p#/proc/}" 2>/dev/null; done
    done
}

# One-shot memory snapshot: the 15 biggest processes by resident memory.
mem_snapshot() {
    {
        echo "$(date '+%Y-%m-%d %H:%M:%S') [mem] $1: $(grep -E '^(MemFree|Buffers|Cached):' /proc/meminfo | tr -s ' ' | tr '\n' ' ')"
        echo "  processes: $(ls -d /proc/[0-9]* | wc -l); biggest by RSS (kB):"
        for st in /proc/[0-9]*/status; do
            awk '/^Name:/{n=$2} /^VmRSS:/{r=$2} END{if (r) print r, n}' "$st" 2>/dev/null
        done | sort -rn | head -15 | sed 's/^/    /'
    } >> "$LOG" 2>&1
}

CPUFREQ=/sys/devices/system/cpu/cpu0/cpufreq
# Run the CPU at full speed while we're in use: Amazon's ondemand setup stayed at 396 MHz under load.
cpu_fast() {
    [ -w "$CPUFREQ/scaling_governor" ] || return
    OLD_GOVERNOR=$(cat "$CPUFREQ/scaling_governor")
    log "cpufreq: governor=$OLD_GOVERNOR min=$(cat $CPUFREQ/scaling_min_freq) max=$(cat $CPUFREQ/scaling_max_freq) avail=[$(cat $CPUFREQ/scaling_available_frequencies 2>/dev/null)] hw_max=$(cat $CPUFREQ/cpuinfo_max_freq)"
    cat "$CPUFREQ/cpuinfo_max_freq" > "$CPUFREQ/scaling_max_freq" 2>/dev/null
    echo performance > "$CPUFREQ/scaling_governor" 2>/dev/null
    log "cpufreq: now $(cat $CPUFREQ/scaling_governor) @ $(cat $CPUFREQ/scaling_cur_freq) kHz"
}

cpu_restore() {
    [ -n "$OLD_GOVERNOR" ] && echo "$OLD_GOVERNOR" > "$CPUFREQ/scaling_governor" 2>/dev/null
}

kill_chroot_procs() {
    for sig in TERM KILL; do
        for p in /proc/[0-9]*; do
            root=$(readlink "$p/root" 2>/dev/null)
            case "$root" in "$R"|"$R"/*) kill -$sig "${p#/proc/}" 2>/dev/null ;; esac
        done
        sleep 1
    done
}

umount_all() {
    kill_chroot_procs
    for d in root etc/dropbear etc var/lib/kos kindle/sys kindle/proc kindle/dev kindle/var kindle tmp run mnt/us dev/pts dev sys proc; do
        umount "$R/$d" 2>/dev/null || umount -l "$R/$d" 2>/dev/null
    done
    umount "$R" 2>/dev/null || umount -l "$R" 2>/dev/null
    # detach the loop device so it isn't leaked (retry while a lazy unmount finishes)
    if [ -f "$LOOPFILE" ]; then
        l=$(cat "$LOOPFILE")
        i=0
        while [ $i -lt 10 ]; do
            losetup -d "$l" 2>/dev/null && break
            sleep 1
            i=$((i + 1))
        done
        [ $i -ge 10 ] && log "warning: couldn't detach $l (still busy)"
        rm -f "$LOOPFILE"
    fi
}

restore_kindle() {
    log "restoring stock Kindle services: $STOPPED"
    # restart in reverse order of stopping (STOPPED is already reversed)
    # volumd (USB drive mode) goes LAST: with a cable attached it immediately hands /mnt/us
    # to the computer, after which nothing on /mnt/us (including this log) is reachable.
    for j in wifim wifis wifid $AMAZON_EXTRA x lab126_gui volumd; do
        case " $STOPPED " in *" $j "*) start "$j" >/dev/null 2>&1; log "started $j" ;; esac
    done
    lipc-set-prop com.lab126.powerd preventScreenSaver 0 >/dev/null 2>&1
}

cleanup() {
    umount_all
    cpu_restore
    # Shutting down or restarting: don't bring Amazon's UI, X and USB drive mode back first (that
    # took most of a minute, and shutdown then had to stop them all again). They start normally
    # on the next boot.
    case "$ACTION" in
        reboot|poweroff) log "skipping stock Kindle restore ($ACTION)" ;;
        *) restore_kindle ;;
    esac
    rmdir "$LOCK" 2>/dev/null
    log "done (action=$ACTION)"
    case "$ACTION" in
        reboot) sync; reboot ;;
        poweroff)
            sync
            # the e-ink panel keeps its last picture once the power is off
            [ -x /mnt/us/libkh/bin/fbink ] && { /mnt/us/libkh/bin/fbink -q -c -f >/dev/null 2>&1
                /mnt/us/libkh/bin/fbink -q -m -M -S 3 "Kindle is off" >/dev/null 2>&1 ||
                    /mnt/us/libkh/bin/fbink -q -m -y 18 -S 3 "Kindle is off" >/dev/null 2>&1
                /mnt/us/libkh/bin/fbink -q -m -y -8 "Hold the power button to turn it on" >/dev/null 2>&1; }
            poweroff ;;
        browser)
            # hand the page to Amazon's experimental browser once the stock UI is up
            url=$(sed -n 2p "$KOS/request" 2>/dev/null)
            log "opening Amazon's browser ${url:+at $url}"
            ( sleep 12
              if [ -n "$url" ]; then
                  lipc-set-prop com.lab126.appmgrd start "app://com.lab126.browser?url=$url" ||
                  lipc-set-prop com.lab126.appmgrd start app://com.lab126.browser
              else
                  lipc-set-prop com.lab126.appmgrd start app://com.lab126.browser
              fi ) >/dev/null 2>&1 &
            ;;
    esac
}

ACTION=exit
trap 'cleanup; exit 0' INT TERM HUP

msg "Starting Kindle Linux..."
if ! mount_all; then
    # a compressed image this kernel can't mount: fall back to an older ext3 image if one is there
    if [ "$IMG" != "$KOS/kos-rootfs.ext3" ] && [ -f "$KOS/kos-rootfs.ext3" ]; then
        umount_all
        log "squashfs image didn't mount; using the old ext3 image"
        IMG=$KOS/kos-rootfs.ext3
        mount_all || { umount_all; rmdir "$LOCK"; msg "Kindle Linux: could not mount image (see kos.log)"; exit 1; }
    else
        umount_all; rmdir "$LOCK"; msg "Kindle Linux: could not mount image (see kos.log)"; exit 1
    fi
elif [ "$IMG" = "$KOS/kos-rootfs.sqsh" ] && [ -f "$KOS/kos-rootfs.ext3" ]; then
    # the compressed image works: the old 256 MB ext3 image is no longer needed
    rm -f "$KOS/kos-rootfs.ext3" && log "removed the old ext3 image (replaced by kos-rootfs.sqsh)"
fi

# Stop Amazon's UI and the services Kindle Linux replaces. powerd stays for safe suspend.
stop_job lab126_gui
# Started from the boot menu: the boot hook already stopped the stock UI, but "Exit to stock
# Kindle" must still bring it back.
if [ "$KOS_BOOT" = "1" ]; then
    case " $STOPPED " in *" lab126_gui "*) ;; *) STOPPED="lab126_gui $STOPPED" ;; esac
    log "started from boot menu"
fi
sleep 2
# Amazon's wifi manager + its wpa_supplicant/dhcp are replaced by ours. wifim stays: it owns the
# ath6kl driver and chip power, and stopping it removes wlan0.
stop_job wifid
stop_job wifis
stop_job volumd          # no USB drive mode while /mnt/us is in use
for j in $AMAZON_EXTRA; do stop_job "$j"; done
stop_job x               # Amazon's X server + awesome WM: we draw straight to the framebuffer
killall wpa_supplicant udhcpc 2>/dev/null
log "network interfaces: $(ls /sys/class/net | tr '\n' ' ')"
cpu_fast
mem_snapshot "after stopping Amazon services"

crashes=0
while :; do
    rm -f "$KOS/request"
    chroot "$R" /opt/kos/run.sh >> "$LOG" 2>&1
    code=$?
    log "gui exited with $code"
    case "$code" in
        0) ACTION=exit; break ;;
        3) continue ;;
        11) ACTION=browser; break ;;
        20) ACTION=reboot; break ;;
        21) ACTION=poweroff; break ;;
        *)
            crashes=$((crashes + 1))
            [ "$crashes" -ge 3 ] && { log "too many crashes, giving up"; break; }
            sleep 2 ;;
    esac
done

cleanup
exit 0
