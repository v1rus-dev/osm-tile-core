// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "OsmTileCore",
    platforms: [
        .iOS(.v13)
    ],
    products: [
        .library(
            name: "OsmTileCore",
            targets: ["OsmTileCore"]
        )
    ],
    targets: [
        .target(
            name: "OsmTileCore",
            dependencies: ["OsmTileCoreFFI"],
            path: "Sources/OsmTileCore"
        ),
        .binaryTarget(
            name: "OsmTileCoreFFI",
            path: "OsmTileCore.xcframework"
        )
    ]
)
