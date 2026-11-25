#!/bin/sh
# Copy Folio to a Kindle mounted as a USB drive (default /Volumes/Kindle).
set -e
cd "$(dirname "$0")/../../.."
K="${1:-/Volumes/Kindle}"
[ -d "$K/documents" ] || { echo "No Kindle at $K"; exit 1; }
[ -f out/kos-rootfs.sqsh ] && [ -f out/kos ] || { echo "Run platforms/kindle/build/build.sh first"; exit 1; }
mkdir -p "$K/kos"
cp out/kos-rootfs.sqsh out/kos platforms/kindle/host/kos-launch.sh platforms/kindle/host/kos-boot.sh platforms/kindle/host/folio.conf "$K/kos/"
cp platforms/kindle/host/Folio.sh platforms/kindle/host/Folio-BootOn.sh platforms/kindle/host/Folio-BootOff.sh "$K/documents/"
# files from before the rename to Folio (an enabled boot hook keeps working: kos-boot.sh switches
# it to folio.conf on the next boot)
rm -f "$K/documents/KindleLinux.sh" "$K/documents/KindleLinux-BootOn.sh" "$K/documents/KindleLinux-BootOff.sh" \
      "$K/kos/kindle-linux.conf"
sync
for f in kos-rootfs.sqsh kos kos-launch.sh kos-boot.sh folio.conf; do
    cmp -s "out/$f" "$K/kos/$f" 2>/dev/null || cmp -s "platforms/kindle/host/$f" "$K/kos/$f" || { echo "verify failed: $f"; exit 1; }
done
for f in Folio.sh Folio-BootOn.sh Folio-BootOff.sh; do
    cmp -s "platforms/kindle/host/$f" "$K/documents/$f" || { echo "verify failed: documents/$f"; exit 1; }
done
command -v dot_clean >/dev/null && dot_clean -m "$K/kos" "$K/documents" 2>/dev/null || true
# (an older kos-rootfs.ext3 is left in place: the launcher removes it after the new image has started once)
echo "Installed and verified on $K"
