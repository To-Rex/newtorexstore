#!/usr/bin/env bash
# Build native Rust libraries for the current platform.
# Usage: ./scripts/build_native.sh [platform]
# Platforms: android, ios, macos, linux, windows, all, current
#
# Run from TorexLocalStore/ directory:
#   cd TorexLocalStore && ./scripts/build_native.sh macos

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STORE_DIR="$(dirname "$SCRIPT_DIR")"
RUST_DIR="$STORE_DIR/rust"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()  { echo -e "${BLUE}[INFO]${NC}  $1"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $1"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $1"; }
fail()  { echo -e "${RED}[FAIL]${NC}  $1"; exit 1; }

PLATFORM="${1:-current}"

echo "🔧 TorexLocalStore Native Build"
echo "   Platform: $PLATFORM"
echo "   Rust dir: $RUST_DIR"
echo ""

# ── Rust toolchain detection ───────────────────────────────────────────────
# Prefer the rustup-managed toolchain over Homebrew's rustc.
# Homebrew rustc lacks iOS/Android cross-compilation std libraries.
if command -v rustup &>/dev/null; then
    RUSTUP_BIN="$(command -v rustup)"
    RUSTUP_CARGO="$(dirname "$RUSTUP_BIN")/cargo"
    RUSTUP_RUSTC="$($RUSTUP_BIN which rustc 2>/dev/null || true)"

    if [ -n "$RUSTUP_RUSTC" ]; then
        export RUSTC="$RUSTUP_RUSTC"
        CARGO_CMD="$RUSTUP_CARGO"
        ok "Using rustup rustc: $RUSTC"
    else
        CARGO_CMD="cargo"
        warn "rustup found but 'rustup which rustc' failed — using PATH cargo"
    fi
elif command -v cargo &>/dev/null; then
    CARGO_CMD="cargo"
    warn "rustup not found — using PATH cargo (cross-compilation may fail)"
else
    fail "Rust not installed. Install via rustup: https://rustup.rs/"
fi

ok "Rust: $("$CARGO_CMD" --version)"

# iOS / macOS SDK paths
XCRUN_IOS_SIM_SDK="$(xcrun --sdk iphonesimulator --show-sdk-path 2>/dev/null || true)"
XCRUN_IOS_SDK="$(xcrun --sdk iphoneos --show-sdk-path 2>/dev/null || true)"
XCRUN_MACOS_SDK="$(xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"

# Deployment targets
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-16.0}"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-12.0}"

build_android() {
    info "Building Android native libraries..."

    if ! command -v cargo-ndk &>/dev/null; then
        info "Installing cargo-ndk..."
        cargo install cargo-ndk
    fi

    if [ -z "${ANDROID_NDK_HOME:-}" ]; then
        warn "ANDROID_NDK_HOME not set. Android build may fail."
        warn "Set ANDROID_NDK_HOME to your NDK path."
    fi

    local JNI_DIR="$STORE_DIR/android/src/main/jniLibs"
    mkdir -p "$JNI_DIR/arm64-v8a"
    mkdir -p "$JNI_DIR/armeabi-v7a"
    mkdir -p "$JNI_DIR/x86_64"

    cd "$RUST_DIR"

    info "  Building arm64-v8a..."
    cargo ndk -t arm64-v8a build --release
    cp "target/aarch64-linux-android/release/libtorex_local_store.so" \
       "$JNI_DIR/arm64-v8a/"

    info "  Building armeabi-v7a..."
    cargo ndk -t armeabi-v7a build --release
    cp "target/armv7-linux-androideabi/release/libtorex_local_store.so" \
       "$JNI_DIR/armeabi-v7a/"

    info "  Building x86_64..."
    cargo ndk -t x86_64 build --release
    cp "target/x86_64-linux-android/release/libtorex_local_store.so" \
       "$JNI_DIR/x86_64/"

    ok "Android build complete"
}

build_ios() {
    info "Building iOS native libraries..."

    local IOS_DIR="$STORE_DIR/ios/Classes"
    mkdir -p "$IOS_DIR"

    cd "$RUST_DIR"

    # iOS device (arm64)
    info "  Building aarch64-apple-ios (device)..."
    SDKROOT="$XCRUN_IOS_SDK" \
        "$CARGO_CMD" build --release --target aarch64-apple-ios

    # iOS simulator (arm64 — Apple Silicon Mac)
    info "  Building aarch64-apple-ios-sim (simulator arm64)..."
    SDKROOT="$XCRUN_IOS_SIM_SDK" \
        "$CARGO_CMD" build --release --target aarch64-apple-ios-sim

    # iOS simulator (x86_64 — Intel Mac)
    info "  Building x86_64-apple-ios (simulator x86_64)..."
    SDKROOT="$XCRUN_IOS_SIM_SDK" \
        "$CARGO_CMD" build --release --target x86_64-apple-ios

    info "  Creating universal simulator library (arm64 + x86_64)..."
    lipo -create \
        "target/aarch64-apple-ios-sim/release/libtorex_local_store.a" \
        "target/x86_64-apple-ios/release/libtorex_local_store.a" \
        -output "$IOS_DIR/libtorex_local_store.a"

    ok "iOS build complete ✔"
    ok "  Simulator (universal): $IOS_DIR/libtorex_local_store.a"
    ok "  Device:                target/aarch64-apple-ios/release/libtorex_local_store.a"
    info "  Tip: for device+simulator in one artifact, run scripts/build_xcframework.sh"
}

build_macos() {
    info "Building macOS native library..."

    local MACOS_DIR="$STORE_DIR/macos/Classes"
    mkdir -p "$MACOS_DIR"

    cd "$RUST_DIR"

    info "  Building aarch64-apple-darwin (Apple Silicon)..."
    cargo build --release --target aarch64-apple-darwin

    info "  Building x86_64-apple-darwin (Intel)..."
    cargo build --release --target x86_64-apple-darwin

    info "  Creating universal library..."
    lipo -create \
        "target/aarch64-apple-darwin/release/libtorex_local_store.a" \
        "target/x86_64-apple-darwin/release/libtorex_local_store.a" \
        -output "$MACOS_DIR/libtorex_local_store.a"

    ok "macOS build complete"
}

build_linux() {
    info "Building Linux native library..."

    local LINUX_DIR="$STORE_DIR/linux/lib"
    mkdir -p "$LINUX_DIR"

    cd "$RUST_DIR"

    info "  Building x86_64-unknown-linux-gnu..."
    cargo build --release

    cp "target/release/libtorex_local_store.a" "$LINUX_DIR/"

    ok "Linux build complete"
}

build_windows() {
    info "Building Windows native library..."

    local WIN_DIR="$STORE_DIR/windows/lib"
    mkdir -p "$WIN_DIR"

    cd "$RUST_DIR"

    info "  Building x86_64-pc-windows-msvc..."
    cargo build --release --target x86_64-pc-windows-msvc

    cp "target/x86_64-pc-windows-msvc/release/torex_local_store.lib" "$WIN_DIR/"

    ok "Windows build complete"
}

build_current() {
    local OS="$(uname -s)"
    case "$OS" in
        Darwin)
            build_macos
            ;;
        Linux)
            build_linux
            ;;
        MINGW*|MSYS*|CYGWIN*)
            build_windows
            ;;
        *)
            fail "Unsupported OS: $OS"
            ;;
    esac
}

case "$PLATFORM" in
    android)
        build_android
        ;;
    ios)
        build_ios
        ;;
    macos)
        build_macos
        ;;
    linux)
        build_linux
        ;;
    windows)
        build_windows
        ;;
    current)
        build_current
        ;;
    all)
        info "Building ALL platforms..."
        build_android || warn "Android build failed (requires NDK)"
        build_ios     || warn "iOS build failed (requires macOS)"
        build_macos   || warn "macOS build failed (requires macOS)"
        build_linux   || warn "Linux build failed"
        build_windows || warn "Windows build failed"
        ;;
    *)
        fail "Unknown platform: $PLATFORM\nUsage: $0 [android|ios|macos|linux|windows|all|current]"
        ;;
esac

echo ""
ok "🎉 Native build complete!"
