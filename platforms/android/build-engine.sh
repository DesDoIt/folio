#!/bin/sh
# Build the shared engine for Android and generate its Kotlin API (UniFFI).
# Gradle runs this before every app build; it can also be run by hand.
# Output: platforms/android/app/build/engine/{jniLibs,kotlin}
set -e
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH"
if [ -z "$ANDROID_NDK_HOME" ]; then
    sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
    ANDROID_NDK_HOME=$(ls -d "$sdk"/ndk/* 2>/dev/null | sort -V | tail -1)
    export ANDROID_NDK_HOME
fi
[ -d "$ANDROID_NDK_HOME" ] || { echo "Android NDK not found: install it with sdkmanager \"ndk;27.2.12479018\"" >&2; exit 1; }
OUT=platforms/android/app/build/engine
ABIS=${FOLIO_ABIS:-"arm64-v8a x86_64"}
targets=""
for a in $ABIS; do targets="$targets -t $a"; done
cargo ndk $targets -P 24 -o "$OUT/jniLibs" build --release -p engine-ffi
# The Kotlin API is generated from a host build of the same crate.
cargo build -q -p engine-ffi
case "$(uname)" in Darwin) lib=target/debug/libengine_ffi.dylib ;; *) lib=target/debug/libengine_ffi.so ;; esac
rm -rf "$OUT/kotlin"
cargo run -q -p uniffi-bindgen -- generate --library "$lib" --language kotlin --no-format --out-dir "$OUT/kotlin"
