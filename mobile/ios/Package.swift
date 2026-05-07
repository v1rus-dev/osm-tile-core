// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "OsmTileEngine",
    platforms: [
        .iOS(.v13)
    ],
    products: [
        .library(
            name: "OsmTileEngine",
            targets: ["OsmTileEngine"]
        )
    ],
    targets: [
        .target(
            name: "OsmTileEngine",
            dependencies: ["OsmTileEngineFFI"],
            path: "Sources/OsmTileEngine"
        ),
        .binaryTarget(
            name: "OsmTileEngineFFI",
            path: "OsmTileEngine.xcframework"
        )
    ]
)
