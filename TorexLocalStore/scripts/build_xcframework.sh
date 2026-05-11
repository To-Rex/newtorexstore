#!/usr/bin/env bash
# Build XCFramework for iOS (device + simulator)
# Usage: ./scripts/build_xcframework.sh
# Run from TorexLocalStore/ directory

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STORE_DIR="$(dirname "$SCRIPT_DIR")"
RUST_DIR="$STORE_DIR/rust"
XCFRAMEWORK_DIR="$STORE_DIR/ios/Frameworks"

echo "Building iOS XCFramework..."

mkdir -p "$XCFRAMEWORK_DIR"

cd "$RUST_DIR"

# Build device
cargo build --release --target aarch64-apple-ios

# Build simulator universal
cargo build --release --target aarch64-apple-ios-sim
cargo build --release --target x86_64-apple-ios

DEVICE_LIB="target/aarch64-apple-ios/release/libtorex_local_store.a"
SIM_LIB="target/aarch64-apple-ios-sim/release/libtorex_local_store.a"
X86_SIM_LIB="target/x86_64-apple-ios/release/libtorex_local_store.a"

SIM_UNIVERSAL="$XCFRAMEWORK_DIR/libtorex_local_store-sim.a"

lipo -create "$SIM_LIB" "$X86_SIM_LIB" -output "$SIM_UNIVERSAL"

xcodebuild -create-xcframework \
    -library "$DEVICE_LIB" \
    -library "$SIM_UNIVERSAL" \
    -output "$XCFRAMEWORK_DIR/TorexLocalStore.xcframework"

rm -f "$SIM_UNIVERSAL"

echo "XCFramework created at $XCFRAMEWORK_DIR/TorexLocalStore.xcframework"
