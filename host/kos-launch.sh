#!/bin/sh
# Kindle Linux launcher - runs on the Kindle (host side, busybox ash).
# Mounts the Alpine image, stops Amazon's UI and services, runs the GUI in a chroot,
# then ALWAYS tears everything down and restores the stock Kindle.
#
# Escape hatches:
#   * create /mnt/us/kos/disable to stop kindle-os from starting at all
#   * hold the power button ~15 s to force a reboot (stock Kindle comes back)

KOS=/mnt/us/kos
IMG=$KOS/kos-rootfs.ext3
R=/var/tmp/kos-root
LOG=$KOS/kos.log
LOCK=/var/run/kos.lock
STOPPED=""
# Amazon reader/indexer/updater services. They don't stop with lab126_gui, and KOReader's
# launcher restarts all of them when it exits (it assumes it returns to Amazon's UI).
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
    if command -v e2fsck >/dev/null 2>&1; then
        e2fsck -p "$LOOP" >> "$LOG" 2>&1
    fi
    if ! mount -t ext3 -o noatime "$LOOP" "$R" 2>>"$LOG"; then
        log "mount image failed on $LOOP; kernel says: $(dmesg | tail -4 | tr '\n' ' ')"
        losetup -d "$LOOP" 2>/dev/null
        rm -f "$LOOPFILE"
        return 1
    fi
    log "image mounted via $LOOP"
    for d in proc sys dev dev/pts mnt/us kindle; do mkdir -p "$R/$d"; done
    mount -t proc proc "$R/proc"
    mount -t sysfs sys "$R/sys"
    mount -o bind /dev "$R/dev"
    mount -o bind /dev/pts "$R/dev/pts"
    mount -o bind /mnt/us "$R/mnt/us"
    # Kindle root, read-only, for lipc/iptables helpers (chroot /kindle ...)
    mount -o bind / "$R/kindle" && mount -o remount,ro,bind "$R/kindle" 2>/dev/null
    for d in var dev proc sys; do mount -o bind "/$d" "$R/kindle/$d" 2>/dev/null; done
    mkdir -p "$R/run" "$R/tmp"
    mount -t tmpfs -o size=16m tmpfs "$R/run"
    mount -t tmpfs -o size=32m tmpfs "$R/tmp"
    return 0
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

# Log CPU/memory while KOReader runs, so slowdowns can be diagnosed from kos.log.
perf_monitor() {
    i=0
    while [ $i -lt 8 ]; do
        sleep 20
        {
            echo "$(date '+%Y-%m-%d %H:%M:%S') [perf] load: $(cat /proc/loadavg)"
            echo "  cpufreq: $(cat $CPUFREQ/scaling_governor 2>/dev/null) $(cat $CPUFREQ/scaling_cur_freq 2>/dev/null)kHz (max $(cat $CPUFREQ/scaling_max_freq 2>/dev/null))"
            grep -E '^(MemFree|Buffers|Cached):' /proc/meminfo | tr -s ' ' | sed 's/^/  /'
            echo "  processes: $(ls -d /proc/[0-9]* | wc -l); biggest by RSS (kB):"
            # busybox top has no batch mode on this firmware: read RSS straight from /proc
            for st in /proc/[0-9]*/status; do
                awk '/^Name:/{n=$2} /^VmRSS:/{r=$2} END{if (r) print r, n}' "$st" 2>/dev/null
            done | sort -rn | head -15 | sed 's/^/    /'
        } >> "$LOG" 2>&1
        i=$((i + 1))
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

koreader_session() {
    file="$1"
    # KOReader talks to Amazon's wifi manager over lipc; with it stopped those calls stall the UI.
    # Hand Wi-Fi back to Amazon while KOReader runs.
    kill_chroot_named wpa_supplicant udhcpc
    start wifis >/dev/null 2>&1
    start wifid >/dev/null 2>&1
    # Run KOReader in its "no framework" mode: it then skips its lipc calls to Amazon's (stopped)
    # UI, which otherwise time out for ~10 s on start and exit. That mode ends with
    # `start lab126_gui`; a PATH shim turns just that one call into a no-op.
    SHIM=/var/tmp/kos-shim
    REAL_START=$(command -v start)
    mkdir -p "$SHIM"
    printf '#!/bin/sh\n[ "$1" = lab126_gui ] && exit 0\nexec %s "$@"\n' "$REAL_START" > "$SHIM/start"
    chmod +x "$SHIM/start"
    log "launching KOReader ${file:+with $file} (no-framework mode)"
    perf_monitor &
    mon=$!
    ( cd /mnt/us && export PATH="$SHIM:$PATH" && \
      if [ -n "$file" ]; then sh koreader/koreader.sh --framework_stop "$file"; \
      else sh koreader/koreader.sh --framework_stop; fi ) >> "$KOS/koreader.log" 2>&1
    log "KOReader exited ($?)"
    kill $mon 2>/dev/null
    # KOReader just restarted Amazon's services; stop them again (restored on final exit)
    for j in $AMAZON_EXTRA; do
        if status "$j" 2>/dev/null | grep -q "start/"; then
            stop "$j" >/dev/null 2>&1
            case " $STOPPED " in *" $j "*) ;; *) STOPPED="$j $STOPPED" ;; esac
        fi
    done
    stop wifid >/dev/null 2>&1
    stop wifis >/dev/null 2>&1
    killall wpa_supplicant udhcpc 2>/dev/null
    log "back from KOReader; stopped Amazon services again"
    [ -n "$OLD_GOVERNOR" ] && echo performance > "$CPUFREQ/scaling_governor" 2>/dev/null
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
    for d in kindle/sys kindle/proc kindle/dev kindle/var kindle tmp run mnt/us dev/pts dev sys proc; do
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
    restore_kindle
    rmdir "$LOCK" 2>/dev/null
    log "done (action=$ACTION)"
    case "$ACTION" in
        reboot) sync; reboot ;;
        poweroff) sync; poweroff ;;
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
    umount_all; rmdir "$LOCK"; msg "Kindle Linux: could not mount image (see kos.log)"; exit 1
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
stop_job x               # Amazon's X server + awesome WM: neither we nor KOReader draw through X
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
        10)
            koreader_session "$(sed -n 2p "$KOS/request" 2>/dev/null)"
            crashes=0
            continue ;;
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
