// swift-tools-version:5.9
import PackageDescription

// NOTE: The binary target below points at the XCFramework asset attached to
// a tagged GitHub release. The checksum placeholder is patched by the
// release workflow (.github/workflows/release.yml) before the tag is cut,
// so every *tagged* version resolves correctly via SwiftPM.
//
// On untagged commits (including main before the first release) the binary
// target is NOT resolvable — depend on a tagged version:
//
//   .package(url: "https://github.com/renedeanda/cindermark", from: "0.2.0")

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
            url: "https://github.com/renedeanda/cindermark/releases/download/v0.2.0/CindermarkFFI.xcframework.zip",
            checksum: "5020485ef10eb26fe0dd4fb0bf0749deb9fcc6c44b0ada934e3c43adf1d9d6f9" // PLACEHOLDER — patched by release.yml
        ),
        .target(
            name: "Cindermark",
            dependencies: ["CindermarkFFIFFI"],
            path: "swift/Sources/Cindermark"
        ),
    ]
)
