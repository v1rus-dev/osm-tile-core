# Repository Guidelines

## Project Structure & Module Organization

`osm-tile-core` is a Rust 2024 library for tile loading, caching, map state, and mobile bindings.

- `src/lib.rs` exports the public API.
- `src/cache.rs`, `source.rs`, `tile_id.rs`, `geo.rs`, `map_state.rs`, and `marker.rs` hold the core domain modules.
- `src/mobile.rs` contains UniFFI-facing types and is compiled with the `mobile` feature.
- `src/bin/` contains UniFFI binding generator entry points.
- `examples/fetch_tile.rs` is a runnable cache and tile-fetch example.
- `mobile/android` and `mobile/ios` contain generated binding targets and platform package files.
- `scripts/` contains Android and iOS build scripts; `docs/mobile.md` documents mobile usage.

## Build, Test, and Development Commands

- `cargo build` builds the default Rust library.
- `cargo test` runs unit tests in `src/`.
- `cargo test --features mobile` also compiles and tests the UniFFI mobile API.
- `cargo run --example fetch_tile` fetches tile `0/0/0` from the default local URL and writes cache files under `/tmp/osm-tile-cache`.
- `cargo fmt` formats Rust code with rustfmt.
- `cargo clippy --all-targets --all-features` checks common Rust issues before review.
- `scripts/build-android.sh` builds Android artifacts; requires `cargo-ndk` and `ANDROID_NDK_HOME` or `NDK_HOME`.
- `scripts/build-ios.sh` builds Swift sources and an XCFramework; requires macOS and Xcode.

## Coding Style & Naming Conventions

Use standard Rust formatting: four-space indentation, rustfmt defaults, and grouped imports. Name modules and functions in `snake_case`, public types in `PascalCase`, constants in `SCREAMING_SNAKE_CASE`, and features with short kebab-case names such as `uniffi-cli`. Keep public exports centralized in `src/lib.rs`. Prefer `thiserror` for error types and return `TileError` across public library APIs where appropriate.

## Testing Guidelines

Keep unit tests next to the module they exercise in `mod tests`. Use descriptive test names that state expected behavior, for example `rejects_out_of_range_tile_coordinates`. Async behavior is tested with `#[tokio::test]`. Add or update tests whenever changing coordinate math, cache behavior, URL templating, marker clustering, or UniFFI conversion logic. Run `cargo test --features mobile` before changes that touch `src/mobile.rs` or binding generation.

## Commit & Pull Request Guidelines

The current history uses short, imperative summaries such as `Initial commit`; keep commit subjects concise and action-oriented, for example `Add marker clustering tests`. Pull requests should include the motivation, key implementation details, commands run, and any mobile platform impact. Link related issues when available. Include screenshots only for platform UI changes outside this core crate.

## Security & Configuration Tips

Do not commit generated caches, local tile data, private server URLs, or platform signing material. Pass app-private cache directories from Android and iOS callers. For Android emulator development, use `10.0.2.2` to reach tile servers running on the host machine.
