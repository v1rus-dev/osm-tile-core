#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_DIR="$ROOT_DIR/mobile/android"
JNI_LIBS_DIR="$ANDROID_DIR/src/main/jniLibs"
KOTLIN_DIR="$ANDROID_DIR/src/main/kotlin"
LEGACY_KOTLIN_DIR="$ANDROID_DIR/src/main/java"
GENERATED_PACKAGE_DIR="yegor/cheprasov/osmtileengine"
GENERATED_KOTLIN_FILE="osm_tile_engine.kt"
HOST_LIB="$ROOT_DIR/target/release/libosm_tile_engine.so"
ANDROID_ABIS="${ANDROID_ABIS:-arm64-v8a x86_64}"

clean_generated_bindings() {
  rm -f "$KOTLIN_DIR/$GENERATED_PACKAGE_DIR/$GENERATED_KOTLIN_FILE"
  rm -f "$LEGACY_KOTLIN_DIR/$GENERATED_PACKAGE_DIR/$GENERATED_KOTLIN_FILE"
  find "$KOTLIN_DIR" "$LEGACY_KOTLIN_DIR/yegor" -depth -type d -empty -delete 2>/dev/null || true
}

if [[ "${1:-}" == "clean" ]]; then
  rm -rf "$JNI_LIBS_DIR"
  clean_generated_bindings
  echo "Removed generated Android artifacts"
  exit 0
fi

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

clean_generated_bindings
mkdir -p "$JNI_LIBS_DIR" "$KOTLIN_DIR"

TARGET_ARGS=()
for abi in $ANDROID_ABIS; do
  TARGET_ARGS+=("-t" "$abi")
done

cargo build -p osm-tile-engine --release --features mobile

cargo run -p osm-tile-engine --features uniffi-cli --bin uniffi-bindgen -- \
  generate \
  --library "$HOST_LIB" \
  --language kotlin \
  --out-dir "$KOTLIN_DIR"

cargo ndk \
  "${TARGET_ARGS[@]}" \
  -o "$JNI_LIBS_DIR" \
  build -p osm-tile-engine --release --features mobile,android-renderer

echo "Android artifacts generated in $ANDROID_DIR"
echo "Built Android ABIs: $ANDROID_ABIS"
