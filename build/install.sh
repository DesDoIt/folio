#!/bin/sh
# Copy Kindle Linux to a Kindle mounted as a USB drive (default /Volumes/Kindle).
set -e
cd "$(dirname "$0")/.."
K="${1:-/Volumes/Kindle}"
[ -d "$K/documents" ] || { echo "No Kindle at $K"; exit 1; }
[ -f out/kos-rootfs.ext3 ] && [ -f out/kos ] || { echo "Run build/build.sh first"; exit 1; }
mkdir -p "$K/kos"
cp out/kos-rootfs.ext3 out/kos host/kos-launch.sh host/kos-boot.sh host/kindle-linux.conf "$K/kos/"
cp host/KindleLinux.sh host/KindleLinux-BootOn.sh host/KindleLinux-BootOff.sh "$K/documents/"
sync
for f in kos-rootfs.ext3 kos kos-launch.sh kos-boot.sh kindle-linux.conf; do
    cmp -s "out/$f" "$K/kos/$f" 2>/dev/null || cmp -s "host/$f" "$K/kos/$f" || { echo "verify failed: $f"; exit 1; }
done
command -v dot_clean >/dev/null && dot_clean -m "$K/kos" "$K/documents" 2>/dev/null || true
echo "Installed and verified on $K"
