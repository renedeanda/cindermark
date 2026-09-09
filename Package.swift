// swift-tools-version:5.9
import PackageDescription

// Use a published version tag; preparation branches can reference an
// artifact that has not yet been attached to a GitHub release.

let package = Package(
    name: "Cindermark",
    platforms: [
        .iOS(.v16),
        .macOS(.v13),
    ],
    products: [
        .library(name: "Cindermark", targets: ["Cindermark"])
    ],
    targets: [
        .binaryTarget(
            name: "CindermarkFFIFFI",
            url: "https://github.com/renedeanda/cindermark/releases/download/v0.3.0/CindermarkFFI.xcframework.zip",
            checksum: "58250d32cdd65e2dbb9f3e7f4a902dacbda32f4be045d2b373476820d221fffd"
        ),
        .target(
            name: "Cindermark",
            dependencies: ["CindermarkFFIFFI"],
            path: "swift/Sources/Cindermark"
        ),
    ]
)
