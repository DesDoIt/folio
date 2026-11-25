#!/bin/sh
# Build out/kos-rootfs.sqsh using a native Alpine container (no ARM emulation needed).
cd "$(dirname "$0")/../../.." || exit 1
mkdir -p out
# 1) cross-compile the GUI for the Kindle (armv7, static musl)
platforms/kindle/build/cargo.sh kindle || exit 1
docker run --rm --platform linux/arm64 -v kos-target:/t -v "$PWD/out":/out docker.io/library/alpine:3.24 \
    cp /t/armv7-unknown-linux-musleabihf/release/kos /out/kos || exit 1
# 2) assemble the Alpine root filesystem image
exec docker run --rm --platform linux/arm64 -v "$PWD":/src:ro -v "$PWD/out":/out \
    docker.io/library/alpine:3.24 sh /src/platforms/kindle/build/build-rootfs.sh
