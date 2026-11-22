#!/bin/sh
# Run cargo inside the official Rust image. Usage:
#   build/cargo.sh kindle   -> release binary for the Kindle (armv7 musl, static)
#   build/cargo.sh sim ARGS -> build + run the PNG simulator natively in the container
#   build/cargo.sh <any cargo args>
cd "$(dirname "$0")/.." || exit 1
TARGET=armv7-unknown-linux-musleabihf
case "$1" in
  kindle) shift; CMD="rustup target add $TARGET >/dev/null 2>&1; RUSTFLAGS='-C linker=rust-lld -C link-self-contained=yes' cargo build --release --target $TARGET $*" ;;
  sim) shift; CMD="if [ ! -f /usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf ]; then apt-get -qq update >/dev/null 2>&1; apt-get -qq install -y fonts-droid-fallback >/dev/null 2>&1; fi; command -v mutool >/dev/null || { apt-get -qq update >/dev/null 2>&1; apt-get -qq install -y mupdf-tools djvulibre-bin libarchive-tools >/dev/null 2>&1; }; ls /usr/share/fonts/truetype/droid/; cargo run --release -- --sim $*" ;;
  *) CMD="cargo $*" ;;
esac
exec docker run --rm --platform linux/arm64 \
  -v kos-cargo-registry:/usr/local/cargo/registry -v kos-rustup:/usr/local/rustup \
  -v kos-target:/src/target -v "$PWD":/src -w /src docker.io/library/rust:1 sh -c "$CMD"
