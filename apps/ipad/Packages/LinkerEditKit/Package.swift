// swift-tools-version: 6.2
import PackageDescription

// Everything in the iPad app that can be tested without UI: the /ws protocol, the store, the connection and
// Linker Connect sign-in.
let package = Package(
    name: "LinkerEditKit",
    platforms: [.iOS(.v26), .macOS(.v14)],
    products: [
        .library(name: "LinkerEditKit", targets: ["LinkerEditKit"]),
    ],
    targets: [
        .target(name: "LinkerEditKit", path: "Sources/LinkerEditKit"),
        // Fixtures/ holds real messages recorded from the server (crates/server/src/http/ipad_fixtures.rs).
        .testTarget(
            name: "LinkerEditKitTests",
            dependencies: ["LinkerEditKit"],
            path: "Tests",
            resources: [.copy("Fixtures")]
        ),
    ],
    // Tools 6.2 for .iOS(.v26); the code is still Swift 5 (strict concurrency is a change of its own).
    swiftLanguageModes: [.v5]
)
