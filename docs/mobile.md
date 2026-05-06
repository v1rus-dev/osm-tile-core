# Mobile Usage

`osm-tile-core` exposes Android and iOS bindings through UniFFI. The mobile API
is intentionally smaller than the internal Rust API: use `OsmTileCore` as the
main entry point, pass app-private cache paths from the platform, and let the UI
render markers/clusters with MapLibre, MapKit, or another map view.

## Android

Use `10.0.2.2` when an Android emulator needs to reach a tile server running on
the development machine:

```kotlin
val core = OsmTileCore(
    tileUrlTemplate = "http://10.0.2.2:8080/tile/{z}/{x}/{y}.png",
    cacheDir = context.filesDir.resolve("tile-cache").path
)

val tileBytes = core.loadTile(z = 0, x = 0, y = 0)

core.setViewport(
    MobileViewport(
        south = 53.0,
        west = 27.0,
        north = 54.5,
        east = 28.5,
        zoom = 12
    )
)

core.upsertMarkers(
    listOf(
        MobileMarker(
            id = 1,
            lat = 53.9023,
            lon = 27.5619,
            kind = "poi",
            minZoom = 0,
            maxZoom = 18
        )
    )
)

val visibleMarkers = core.visibleMarkers()
val renderItems = core.clusteredMarkers()
```

`loadTile` is synchronous in v1. Call it from a background dispatcher, not from
the Android UI thread.

## iOS

Use an app-private cache directory, for example inside `cachesDirectory`:

```swift
let cacheDir = FileManager.default.urls(
    for: .cachesDirectory,
    in: .userDomainMask
)[0].appendingPathComponent("tile-cache")

let core = try OsmTileCore(
    tileUrlTemplate: "http://localhost:8080/tile/{z}/{x}/{y}.png",
    cacheDir: cacheDir.path
)

let tileBytes = try core.loadTile(z: 0, x: 0, y: 0)

try core.setViewport(viewport: MobileViewport(
    south: 53.0,
    west: 27.0,
    north: 54.5,
    east: 28.5,
    zoom: 12
))

try core.upsertMarkers(markers: [
    MobileMarker(
        id: 1,
        lat: 53.9023,
        lon: 27.5619,
        kind: "poi",
        minZoom: 0,
        maxZoom: 18
    )
])

let visibleMarkers = try core.visibleMarkers()
let renderItems = try core.clusteredMarkers()
```

`loadTile` is synchronous in v1. Call it away from the main thread, for example
inside a Swift task or background queue.

## Building Bindings

Android:

```bash
scripts/build-android.sh
```

iOS, from macOS with Xcode:

```bash
scripts/build-ios.sh
```

The Rust `///` comments on the UniFFI-exposed mobile types and methods are
included in generated Kotlin and Swift bindings, so Android Studio and Xcode can
show usage hints while editing platform code.
