#!/usr/bin/env bash
#
# build-apple.sh — Build Cindermark for iOS/macOS and generate Swift bindings.
#
# Usage:
#   ./build-apple.sh [mode] [--out-dir <dir>]
#
#   Modes:
#     dev       (default) Build sim + device + macOS slices with the
#               `dev-optimized` Cargo profile — near-production speed,
#               fast rebuilds.
#     debug     Unoptimized artifacts for Rust-level debugging.
#     release   Universal binaries for all 5 Apple targets (slow, ship-ready).
#     bindings  Generate Swift bindings only (no .a rebuild).
#
#   --out-dir defaults to ./out/apple. Host apps that vendor Cindermark
#   (e.g. as a git submodule) pass their own integration directory:
#     ./build-apple.sh release --out-dir "$PROJECT_DIR/Parser"
#
# Prerequisites:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios \
#                     aarch64-apple-darwin x86_64-apple-darwin
#
# Dev mode builds all three slices Xcode looks for. Omitting any slice leaves
# a stale .a in the search path and the running app silently links against
# outdated Rust code — recent changes appear to do nothing even though the
# Swift bindings and source look correct.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$SCRIPT_DIR"
LIB_NAME="libcindermark.a"
SWIFT_SOURCES_DIR="$SCRIPT_DIR/swift/Sources/Cindermark"

MODE="dev"
OUT_DIR="$SCRIPT_DIR/out/apple"

while [[ $# -gt 0 ]]; do
    case "$1" in
        dev|sim|simulator|optimized|dev-optimized)
            MODE="dev"; shift ;;
        debug|dev-debug)
            MODE="debug"; shift ;;
        release)
            MODE="release"; shift ;;
        bindings)
            MODE="bindings"; shift ;;
        --out-dir)
            [[ $# -ge 2 ]] || { echo "--out-dir requires a value" >&2; exit 1; }
            OUT_DIR="$2"; shift 2 ;;
        *)
            echo "Usage: $0 [dev|debug|release|bindings] [--out-dir <dir>]" >&2
            exit 1 ;;
    esac
done

# Ensure output directories exist
mkdir -p "$OUT_DIR" "$OUT_DIR/lib-sim" "$OUT_DIR/lib-device" "$OUT_DIR/lib-macos"

# Detect host architecture once so the default dev build targets the
# simulator + macOS slices that actually match the running machine.
HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
    arm64|aarch64)
        SIM_TARGET="aarch64-apple-ios-sim"
        MACOS_TARGET="aarch64-apple-darwin"
        ;;
    x86_64)
        SIM_TARGET="x86_64-apple-ios"
        MACOS_TARGET="x86_64-apple-darwin"
        ;;
    *)
        echo "Unsupported host architecture: $HOST_ARCH" >&2
        exit 1
        ;;
esac

generate_bindings() {
    echo "==> Generating Swift bindings..."
    # Uses the crate's custom uniffi-bindgen binary (see uniffi-bindgen.rs).
    # Must build it first, then invoke with the UDL file. `bindgen` enables the
    # UniFFI CLI toolchain the binary needs (and implies `ffi`).
    cargo run --manifest-path "$CRATE_DIR/Cargo.toml" --features bindgen \
        --bin uniffi-bindgen generate \
        "$CRATE_DIR/src/cindermark.udl" \
        --language swift \
        --out-dir "$OUT_DIR"
    # Sync header into the Xcode module directory so builds see the latest FFI declarations
    mkdir -p "$OUT_DIR/CindermarkFFIFFI"
    cp "$OUT_DIR/CindermarkFFIFFI.h" "$OUT_DIR/CindermarkFFIFFI/CindermarkFFIFFI.h"
    cat > "$OUT_DIR/CindermarkFFIFFI/module.modulemap" <<'EOF'
module CindermarkFFIFFI {
    header "CindermarkFFIFFI.h"
    export *
}
EOF
    rm -f "$OUT_DIR/module.modulemap"
    # Keep the committed SPM source target in sync with the UDL.
    if [[ -d "$SWIFT_SOURCES_DIR" ]]; then
        cp "$OUT_DIR/CindermarkFFI.swift" "$SWIFT_SOURCES_DIR/CindermarkFFI.swift"
        echo "    -> $SWIFT_SOURCES_DIR/CindermarkFFI.swift (SPM source target)"
    fi
    echo "    -> $OUT_DIR/CindermarkFFI.swift"
    echo "    -> $OUT_DIR/CindermarkFFIFFI.h (+ module copy)"
    echo "    -> $OUT_DIR/CindermarkFFIFFI/module.modulemap"
    echo "    -> $OUT_DIR/CindermarkFFIFFI.modulemap"
}

copy_profile_libs() {
    local PROFILE_DIR="$1"
    local PROFILE_LABEL="$2"

    # Drop the fresh .a into every per-SDK search path Xcode uses, plus the
    # top-level path (kept for anything that historically linked it
    # directly). Missing any path is the common "my changes don't show up
    # in the running app" trap.
    cp "$CRATE_DIR/target/$SIM_TARGET/$PROFILE_DIR/$LIB_NAME"            "$OUT_DIR/lib-sim/$LIB_NAME"
    cp "$CRATE_DIR/target/aarch64-apple-ios/$PROFILE_DIR/$LIB_NAME"      "$OUT_DIR/lib-device/$LIB_NAME"
    cp "$CRATE_DIR/target/$MACOS_TARGET/$PROFILE_DIR/$LIB_NAME"          "$OUT_DIR/lib-macos/$LIB_NAME"
    cp "$CRATE_DIR/target/$SIM_TARGET/$PROFILE_DIR/$LIB_NAME"            "$OUT_DIR/$LIB_NAME"

    echo "    -> $OUT_DIR/lib-sim/$LIB_NAME    (simulator, $HOST_ARCH, $PROFILE_LABEL)"
    echo "    -> $OUT_DIR/lib-device/$LIB_NAME (iOS device, arm64, $PROFILE_LABEL)"
    echo "    -> $OUT_DIR/lib-macos/$LIB_NAME  (macOS, $HOST_ARCH, $PROFILE_LABEL)"
    echo "    -> $OUT_DIR/$LIB_NAME           (legacy path, simulator copy)"
}

build_dev() {
    echo "==> Building dev targets (simulator + device + macOS, optimized)..."

    cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --features ffi --profile dev-optimized --target "$SIM_TARGET"
    cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --features ffi --profile dev-optimized --target aarch64-apple-ios
    cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --features ffi --profile dev-optimized --target "$MACOS_TARGET"

    copy_profile_libs "dev-optimized" "dev-optimized"
}

build_debug() {
    echo "==> Building debug targets (simulator + device + macOS, unoptimized)..."

    cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --features ffi --target "$SIM_TARGET"
    cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --features ffi --target aarch64-apple-ios
    cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --features ffi --target "$MACOS_TARGET"

    copy_profile_libs "debug" "debug"
}

build_release() {
    echo "==> Building release for all targets..."

    # iOS device (arm64)
    cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --features ffi \
        --release --target aarch64-apple-ios

    # iOS simulator (arm64)
    cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --features ffi \
        --release --target aarch64-apple-ios-sim

    # iOS simulator (x86_64, for Intel Macs)
    cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --features ffi \
        --release --target x86_64-apple-ios

    # macOS (arm64)
    cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --features ffi \
        --release --target aarch64-apple-darwin

    # macOS (x86_64)
    cargo build --manifest-path "$CRATE_DIR/Cargo.toml" --features ffi \
        --release --target x86_64-apple-darwin

    local TARGET_DIR="$CRATE_DIR/target"

    # Create universal simulator lib (arm64 + x86_64)
    echo "==> Creating universal simulator library..."
    lipo -create \
        "$TARGET_DIR/aarch64-apple-ios-sim/release/$LIB_NAME" \
        "$TARGET_DIR/x86_64-apple-ios/release/$LIB_NAME" \
        -output "$OUT_DIR/lib-sim/$LIB_NAME"

    # Create universal macOS lib (arm64 + x86_64)
    echo "==> Creating universal macOS library..."
    lipo -create \
        "$TARGET_DIR/aarch64-apple-darwin/release/$LIB_NAME" \
        "$TARGET_DIR/x86_64-apple-darwin/release/$LIB_NAME" \
        -output "$OUT_DIR/lib-macos/$LIB_NAME"

    # Device lib is single-arch
    cp "$TARGET_DIR/aarch64-apple-ios/release/$LIB_NAME" \
        "$OUT_DIR/lib-device/$LIB_NAME"

    echo "    -> $OUT_DIR/lib-sim/$LIB_NAME (simulator, universal)"
    echo "    -> $OUT_DIR/lib-macos/$LIB_NAME (macOS, universal)"
    echo "    -> $OUT_DIR/lib-device/$LIB_NAME (iOS device, arm64)"
}

case "$MODE" in
    dev)
        build_dev
        generate_bindings
        ;;
    debug)
        build_debug
        generate_bindings
        ;;
    release)
        build_release
        generate_bindings
        ;;
    bindings)
        generate_bindings
        ;;
esac

echo "==> Done."
