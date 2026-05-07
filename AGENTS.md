# Repository Guidelines

## Project Structure & Module Organization

`osm-tile-engine` is a Rust 2024 workspace for tile loading, caching, map state, and mobile bindings.

- `crates/osm-core` contains base types and math: `TileId`, `GeoPoint`, `GeoBounds`/`BoundingBox`, `Viewport`, and `MapProjection`. It must not know about networking, disk cache, Android, or GPU rendering.
- `crates/osm-loader` contains tile loading concerns: HTTP sources, cache-first sources, and file cache.
- `crates/osm-renderer` contains map state, marker models, clustering, and viewport/camera-facing logic. It currently prepares render items for platform UIs; native Rust GPU rendering can live here later.
- `crates/osm-tile-engine` contains the UniFFI-facing adapter and mobile binding generator entry points. It should stay thin and depend on lower layers for logic.
- `crates/osm-loader/examples/fetch_tile.rs` is a runnable cache and tile-fetch example.
- `mobile/android` and `mobile/ios` contain generated binding targets and platform package files.
- `scripts/` contains Android and iOS build scripts; `docs/mobile.md` documents mobile usage.

## Build, Test, and Development Commands

- `cargo build --workspace` builds all default workspace crates.
- `cargo test --workspace` runs unit tests across core, loader, and renderer.
- `cargo test -p osm-tile-engine --features mobile` compiles and tests the UniFFI mobile adapter.
- `cargo run -p osm-loader --example fetch_tile` fetches tile `0/0/0` from the default local URL and writes cache files under `/tmp/osm-tile-cache`.
- `cargo fmt --all` formats Rust code with rustfmt.
- `cargo clippy --workspace --all-targets --all-features` checks common Rust issues before review.
- `scripts/build-android.sh` builds Android artifacts; requires `cargo-ndk` and `ANDROID_NDK_HOME` or `NDK_HOME`.
- `scripts/build-ios.sh` builds Swift sources and an XCFramework; requires macOS and Xcode.

## Coding Style & Naming Conventions

Use standard Rust formatting: four-space indentation, rustfmt defaults, and grouped imports. Name modules and functions in `snake_case`, public types in `PascalCase`, constants in `SCREAMING_SNAKE_CASE`, and features with short kebab-case names such as `uniffi-cli`. Keep public exports centralized in each crate's `src/lib.rs`. Prefer `thiserror` for error types. Keep Android as an adapter; core logic belongs in `osm-core`, `osm-loader`, or `osm-renderer`.

## Testing Guidelines

Keep unit tests next to the module they exercise in `mod tests`. Use descriptive test names that state expected behavior, for example `rejects_out_of_range_tile_coordinates`. Async behavior is tested with `#[tokio::test]`. Add or update tests whenever changing coordinate math, cache behavior, URL templating, marker clustering, or UniFFI conversion logic. Run `cargo test -p osm-tile-engine --features mobile` before changes that touch `crates/osm-tile-engine/src/mobile.rs` or binding generation.

## Commit & Pull Request Guidelines

The current history uses short, imperative summaries such as `Initial commit`; keep commit subjects concise and action-oriented, for example `Add marker clustering tests`. Pull requests should include the motivation, key implementation details, commands run, and any mobile platform impact. Link related issues when available. Include screenshots only for platform UI changes outside this core crate.

## Security & Configuration Tips

Do not commit generated caches, local tile data, private server URLs, or platform signing material. Pass app-private cache directories from Android and iOS callers. For Android emulator development, use `10.0.2.2` to reach tile servers running on the host machine.
