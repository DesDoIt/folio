#!/bin/sh
# Builds the Kindle Linux Alpine armv7 root filesystem image without executing ARM code.
# Runs inside a native (arm64) Alpine container. Output: /out/kos-rootfs.sqsh, a compressed,
# read-only squashfs image (the Kindle's own system uses squashfs, so its kernel can mount it).
# The launcher puts /etc in RAM and keeps settings, Wi-Fi and SSH state on /var/local.
set -eu
ALPINE_VER="${ALPINE_VER:-v3.24}"
R=/rootfs
M=https://dl-cdn.alpinelinux.org/alpine/$ALPINE_VER
PKGS="
alpine-baselayout alpine-keys alpine-release apk-tools busybox busybox-suid musl musl-utils ca-certificates ca-certificates-bundle tzdata
curl ncurses-terminfo-base font-droid-nonlatin
wpa_supplicant iw
mupdf-tools djvulibre libarchive-tools
dropbear dropbear-scp openssh-sftp-server
nano htop less
"
apk add --no-cache squashfs-tools alpine-keys >/dev/null
rm -rf "$R"; mkdir -p "$R/etc/apk"
printf '%s/main\n%s/community\n' "$M" "$M" > "$R/etc/apk/repositories"
# Trust the official Alpine signing keys for armv7 (alpine-keys ships every arch's keys)
mkdir -p "$R/etc/apk/keys"; cp /usr/share/apk/keys/armv7/* "$R/etc/apk/keys/"
apk --arch armv7 --root "$R" --initdb --no-scripts --update-cache \
    -X "$M/main" -X "$M/community" add $PKGS
rm -rf "$R/var/cache/apk"/*
# busybox applet symlinks (normally made by a trigger script we can't run here)
for p in $(busybox --list-full); do
  d="$R/$(dirname "$p")"; mkdir -p "$d"
  [ -e "$R/$p" ] || ln -s /bin/busybox "$R/$p"
done
# Basic system config
echo kindle > "$R/etc/hostname"
printf '127.0.0.1 localhost kindle\n' > "$R/etc/hosts"
printf 'nameserver 1.1.1.1\nnameserver 9.9.9.9\n' > "$R/etc/resolv.conf"
ln -sf /usr/share/zoneinfo/UTC "$R/etc/localtime"
# every mount point must exist in the image: it's read-only on the Kindle
mkdir -p "$R/etc/dropbear" "$R/root" "$R/mnt/us" "$R/kindle" "$R/opt/kos" "$R/var/lib/kos" \
         "$R/proc" "$R/sys" "$R/dev/pts" "$R/run" "$R/tmp"
# root has no password until set from Settings; dropbear refuses empty passwords
sed -i 's#^root:[^:]*:#root:*:#' "$R/etc/shadow"
# Our GUI (static Rust binary) + overlay files
cp -a /src/rootfs/. "$R/"
install -m 0755 /out/kos "$R/opt/kos/kos"
# Strip things we don't need to save space
rm -rf "$R/usr/share/man" "$R/usr/share/doc" "$R/var/cache/apk/"*
du -sh "$R"
mkdir -p /out; rm -f /out/kos-rootfs.sqsh /out/kos-rootfs.ext3
# gzip: the compressor every squashfs-capable kernel has (the Kindle's is 3.0)
mksquashfs "$R" /out/kos-rootfs.sqsh -comp gzip -Xcompression-level 9 -b 131072 -noappend -all-root -no-progress -quiet
ls -la /out/kos-rootfs.sqsh
