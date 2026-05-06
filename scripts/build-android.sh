#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_DIR="$ROOT_DIR/mobile/android"
JNI_LIBS_DIR="$ANDROID_DIR/src/main/jniLibs"
KOTLIN_DIR="$ANDROID_DIR/src/main/java"
HOST_LIB="$ROOT_DIR/target/release/libosm_tile_core.so"

command -v cargo >/dev/null || {
  echo "cargo is required" >&2
  exit 1
}

command -v cargo-ndk >/dev/null || {
  echo "cargo-ndk is required. Install it with: cargo install cargo-ndk" >&2
  exit 1
}

if [[ -z "${ANDROID_NDK_HOME:-}" && -z "${NDK_HOME:-}" ]]; then
  echo "ANDROID_NDK_HOME or NDK_HOME must point to the Android NDK" >&2
  exit 1
fi

mkdir -p "$JNI_LIBS_DIR" "$KOTLIN_DIR"

cargo ndk \
  -t arm64-v8a \
  -t x86_64 \
  -o "$JNI_LIBS_DIR" \
  build --release --features mobile

cargo build --release --features mobile

cargo run --features uniffi-cli --bin uniffi-bindgen -- \
  generate \
  --library "$HOST_LIB" \
  --language kotlin \
  --out-dir "$KOTLIN_DIR"

echo "Android artifacts generated in $ANDROID_DIR"
