#!/usr/bin/env bash
# Organize pre-built native libraries into TorexLocalStore package structure.
# This script is called from CI/CD after building all platform binaries.
#
# Run from TorexLocalStore/ directory:
#   cd TorexLocalStore && ./scripts/organize_artifacts.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STORE_DIR="$(dirname "$SCRIPT_DIR")"
ARTIFACTS_DIR="${1:-artifacts}"

echo "🔧 Organizing native libraries..."
echo "   Artifacts: $ARTIFACTS_DIR"
echo "   Target:    $STORE_DIR"

# ============================================================
# Android: .so files → android/src/main/jniLibs/<abi>/
# ============================================================
echo ""
echo "📱 Android..."
ANDROID_JNI="$STORE_DIR/android/src/main/jniLibs"
mkdir -p "$ANDROID_JNI/arm64-v8a"
mkdir -p "$ANDROID_JNI/armeabi-v7a"
mkdir -p "$ANDROID_JNI/x86_64"

if [ -d "$ARTIFACTS_DIR/android-libs" ]; then
    cp "$ARTIFACTS_DIR/android-libs/"*"libtorex_local_store.so" \
       "$ANDROID_JNI/arm64-v8a/" 2>/dev/null || true
    cp "$ARTIFACTS_DIR/android-libs/"*"libtorex_local_store.so" \
       "$ANDROID_JNI/armeabi-v7a/" 2>/dev/null || true
    cp "$ARTIFACTS_DIR/android-libs/"*"libtorex_local_store.so" \
       "$ANDROID_JNI/x86_64/" 2>/dev/null || true
    echo "   ✅ Android native libraries organized"
else
    echo "   ⚠️  No Android artifacts found"
fi

# ============================================================
# iOS: .a files → ios/Classes/
# ============================================================
echo ""
echo "🍎 iOS..."
IOS_CLASSES="$STORE_DIR/ios/Classes"
mkdir -p "$IOS_CLASSES"

if [ -d "$ARTIFACTS_DIR/ios-libs" ]; then
    cp "$ARTIFACTS_DIR/ios-libs/"*.a "$IOS_CLASSES/" 2>/dev/null || true
    echo "   ✅ iOS native libraries organized"
else
    echo "   ⚠️  No iOS artifacts found"
fi

# ============================================================
# macOS: .a file → macos/Classes/
# ============================================================
echo ""
echo "🖥️  macOS..."
MACOS_CLASSES="$STORE_DIR/macos/Classes"
mkdir -p "$MACOS_CLASSES"

if [ -d "$ARTIFACTS_DIR/macos-libs" ]; then
    cp "$ARTIFACTS_DIR/macos-libs/"*.a "$MACOS_CLASSES/" 2>/dev/null || true
    echo "   ✅ macOS native libraries organized"
else
    echo "   ⚠️  No macOS artifacts found"
fi

# ============================================================
# Linux: .a file → linux/lib/
# ============================================================
echo ""
echo "🐧 Linux..."
LINUX_LIB="$STORE_DIR/linux/lib"
mkdir -p "$LINUX_LIB"

if [ -d "$ARTIFACTS_DIR/linux-libs" ]; then
    cp "$ARTIFACTS_DIR/linux-libs/"*.a "$LINUX_LIB/" 2>/dev/null || true
    echo "   ✅ Linux native libraries organized"
else
    echo "   ⚠️  No Linux artifacts found"
fi

# ============================================================
# Windows: .lib file → windows/lib/
# ============================================================
echo ""
echo "🪟 Windows..."
WINDOWS_LIB="$STORE_DIR/windows/lib"
mkdir -p "$WINDOWS_LIB"

if [ -d "$ARTIFACTS_DIR/windows-libs" ]; then
    cp "$ARTIFACTS_DIR/windows-libs/"*.lib "$WINDOWS_LIB/" 2>/dev/null || true
    echo "   ✅ Windows native libraries organized"
else
    echo "   ⚠️  No Windows artifacts found"
fi

# ============================================================
# Generated bindings
# ============================================================
echo ""
echo "🔗 Generated bindings..."
if [ -d "$ARTIFACTS_DIR/generated-bindings" ]; then
    cp -r "$ARTIFACTS_DIR/generated-bindings/"* \
       "$STORE_DIR/lib/src/rust/" 2>/dev/null || true
    echo "   ✅ Generated bindings organized"
else
    echo "   ⚠️  No generated bindings found"
fi

echo ""
echo "🎉 All artifacts organized successfully!"
