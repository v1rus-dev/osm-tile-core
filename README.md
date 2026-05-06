# OSM Tile Engine

Rust workspace for OpenStreetMap tile loading, caching, map state, and mobile
bindings.

## Android bindings generation under WSL

Run the commands from the repository root:

```bash
cd /mnt/d/Projects/RustProjects/osm-tile-engine
```

The recommended path is the Android build script. It builds the host Rust
library for UniFFI metadata, generates Kotlin bindings, and builds Android
`.so` libraries with `cargo-ndk`:

```bash
scripts/build-android.sh
```

By default the script builds `arm64-v8a` and `x86_64`. This is usually enough
for a modern Android phone and a modern emulator. To build a custom ABI set,
pass `ANDROID_ABIS`:

```bash
ANDROID_ABIS="arm64-v8a armeabi-v7a x86_64" scripts/build-android.sh
```

For local device-only development, you can build only `arm64-v8a`:

```bash
ANDROID_ABIS="arm64-v8a" scripts/build-android.sh
```

For an x86_64 emulator only:

```bash
ANDROID_ABIS="x86_64" scripts/build-android.sh
```

Android Studio should then see:

```text
mobile/android/src/main/kotlin/yegor/cheprasov/osmtileengine/osm_tile_engine.kt
mobile/android/src/main/jniLibs/<abi>/libosm_tile_engine.so
```

Install `cargo-ndk` if it is missing:

```bash
cargo install cargo-ndk
```

The script expects `ANDROID_NDK_HOME` or `NDK_HOME` to point to the Android NDK.
For example:

```bash
export ANDROID_NDK_HOME=/mnt/c/Users/<you>/AppData/Local/Android/Sdk/ndk/<version>
```

## Manual UniFFI generation

This project uses UniFFI proc-macro scaffolding, so there is no `.udl` file to
pass to `uniffi-bindgen`. Generate bindings from the compiled host library:

```bash
cargo build -p osm-tile-engine --release --features mobile

cargo run -p osm-tile-engine --features uniffi-cli --bin uniffi-bindgen -- \
  generate \
  --library target/release/libosm_tile_engine.so \
  --language kotlin \
  --out-dir mobile/android/src/main/kotlin
```

Check that Kotlin files were generated:

```bash
find mobile/android/src/main/kotlin -type f
```

## Cleaning generated Android artifacts

Clean generated Android bindings and native libraries. This also removes the old generated Kotlin file from `src/main/java` if it exists:

```bash
scripts/build-android.sh clean
```

Remove Rust build output too:

```bash
cargo clean
```

Full clean:

```bash
scripts/build-android.sh clean
cargo clean
```
