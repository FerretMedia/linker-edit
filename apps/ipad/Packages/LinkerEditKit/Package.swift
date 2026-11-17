// swift-tools-version: 5.10
import PackageDescription

// Everything in the iPad app that can be tested without UI: the /ws protocol, the store, the connection and
// Linker Connect sign-in.
let package = Package(
    name: "LinkerEditKit",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "LinkerEditKit", targets: ["LinkerEditKit"]),
    ],
    targets: [
        .target(name: "LinkerEditKit", path: "Sources/LinkerEditKit"),
        // Fixtures/ holds real messages recorded from the Node server (apps/desktop/scripts/ipad-fixtures.test.ts).
        .testTarget(
            name: "LinkerEditKitTests",
            dependencies: ["LinkerEditKit"],
            path: "Tests",
            resources: [.copy("Fixtures")]
        ),
    ]
)
