#!/bin/sh
# Name: KOS Diagnostics
# Author: kindle-os
# Collects hardware/system info for building kindle-os. Read-only; writes /mnt/us/kos-diag.txt
OUT=/mnt/us/kos-diag.txt
FB=/mnt/us/libkh/bin/fbink
[ -x "$FB" ] && "$FB" -q -y -5 "Collecting diagnostics..." 2>/dev/null
{
s(){ echo; echo "===== $1"; shift; "$@" 2>&1; }
s date date
s uname uname -a
s cmdline cat /proc/cmdline
s cpuinfo cat /proc/cpuinfo
s meminfo cat /proc/meminfo
s df df -h
s mounts cat /proc/mounts
s partitions cat /proc/partitions
s fb-sysfs sh -c 'for f in /sys/class/graphics/fb0/*; do [ -f "$f" ] && echo "$(basename $f): $(cat $f 2>/dev/null | head -3 | tr "\n" " ")"; done'
s fbink-eval "$FB" -e
s input cat /proc/bus/input/devices
s input-sysfs sh -c 'for d in /sys/class/input/event*; do echo "$d: $(cat $d/device/name 2>/dev/null)"; done'
s lsmod lsmod
s modules sh -c 'find /lib/modules -name "*.ko" | head -200'
s modules-dir sh -c 'ls /lib/modules; ls /lib/modules/* | head -50'
s firmware sh -c 'find /lib/firmware /opt/ar6k /usr/lib/firmware -maxdepth 3 2>/dev/null | head -80'
s net-ifaces sh -c 'ls /sys/class/net; ifconfig -a'
s iwconfig iwconfig
s wifi-bins sh -c 'which wpa_supplicant wpa_cli wifid dhcpcd udhcpc iw iwconfig ip; ls /etc/wpa_supplicant* /var/local/wifi* 2>/dev/null'
s wpa-conf sh -c 'ls -la /var/run/wpa_supplicant /etc/wpa_supplicant 2>/dev/null; ps | grep -i wpa'
s initctl initctl list
s upstart-dir ls /etc/upstart
s inittab sh -c 'ls -la /sbin/init /etc/init.d 2>/dev/null; head -40 /etc/upstart/framework.conf /etc/upstart/lab126_gui.conf 2>/dev/null'
s ps ps -ef
s power-supply sh -c 'for d in /sys/class/power_supply/*; do echo "== $d"; for f in $d/*; do [ -f "$f" ] && echo "$(basename $f)=$(cat $f 2>/dev/null)"; done; done'
s gasgauge sh -c 'gasgauge-info -c; gasgauge-info -s; gasgauge-info -l' 
s power-state sh -c 'cat /sys/power/state; ls /sys/power; lipc-get-prop com.lab126.powerd status'
s usb sh -c 'ls /sys/class/udc /sys/devices/platform/*usb* 2>/dev/null; lsmod | grep -iE "g_|usb|file_storage"'
s rtc sh -c 'ls /dev/rtc*; cat /sys/class/rtc/rtc*/name 2>/dev/null'
s iptables iptables -L -n
s busybox sh -c 'busybox | head -40; ls -la /bin/sh'
s libc sh -c 'ls -la /lib/ld-* /lib/libc.so* /lib/libc-* 2>/dev/null'
s lipc sh -c 'ls /usr/bin | grep -i lipc'
s eink-sysfs sh -c 'ls /sys/devices/platform/ | head -60; find /sys -maxdepth 4 -iname "*epdc*" 2>/dev/null | head; find /sys -maxdepth 5 -iname "*waveform*" 2>/dev/null | head'
s dmesg-tail sh -c 'dmesg | tail -150'
} > "$OUT" 2>&1
sync
[ -x "$FB" ] && "$FB" -q -y -5 "Done! Saved kos-diag.txt" 2>/dev/null
