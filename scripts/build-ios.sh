#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "iOS builds require macOS with Xcode installed" >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IOS_DIR="$ROOT_DIR/mobile/ios"
SWIFT_OUT="$IOS_DIR/Sources/OsmTileCore"
HEADERS_OUT="$IOS_DIR/build/Headers"
XCFRAMEWORK_OUT="$IOS_DIR/OsmTileCore.xcframework"
DEVICE_TARGET="aarch64-apple-ios"
SIM_TARGET="aarch64-apple-ios-sim"
DEVICE_LIB="$ROOT_DIR/target/$DEVICE_TARGET/release/libosm_tile_core.a"
SIM_LIB="$ROOT_DIR/target/$SIM_TARGET/release/libosm_tile_core.a"

command -v cargo >/dev/null || {
  echo "cargo is required" >&2
  exit 1
}

command -v xcodebuild >/dev/null || {
  echo "xcodebuild is required" >&2
  exit 1
}

rustup target add "$DEVICE_TARGET" "$SIM_TARGET"

cargo build --release --features mobile --target "$DEVICE_TARGET"
cargo build --release --features mobile --target "$SIM_TARGET"

rm -rf "$SWIFT_OUT" "$HEADERS_OUT" "$XCFRAMEWORK_OUT"
mkdir -p "$SWIFT_OUT" "$HEADERS_OUT"

cargo run --features uniffi-cli --bin uniffi-bindgen-swift -- \
  "$DEVICE_LIB" "$SWIFT_OUT" --swift-sources

cargo run --features uniffi-cli --bin uniffi-bindgen-swift -- \
  "$DEVICE_LIB" "$HEADERS_OUT" --headers

cargo run --features uniffi-cli --bin uniffi-bindgen-swift -- \
  "$DEVICE_LIB" "$HEADERS_OUT" --xcframework --modulemap --modulemap-filename module.modulemap

xcodebuild -create-xcframework \
  -library "$DEVICE_LIB" -headers "$HEADERS_OUT" \
  -library "$SIM_LIB" -headers "$HEADERS_OUT" \
  -output "$XCFRAMEWORK_OUT"

echo "iOS Swift sources generated in $SWIFT_OUT"
echo "iOS XCFramework generated at $XCFRAMEWORK_OUT"
