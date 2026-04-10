#!/usr/bin/env bash
# build.sh — Configure and build Pier-X without launching it.
#
# Usage:
#   ./build.sh                                # Release build
#   BUILD_TYPE=Debug ./build.sh               # Debug build
#   QT_DIR=~/Qt/6.8.1/macos ./build.sh        # Use a specific Qt install
#   BUILD_DIR=build-debug ./build.sh          # Custom build directory

set -euo pipefail

cd "$(dirname "$0")"

BUILD_TYPE="${BUILD_TYPE:-Release}"
BUILD_DIR="${BUILD_DIR:-build}"

CMAKE_ARGS=(-B "$BUILD_DIR" -S . -DCMAKE_BUILD_TYPE="$BUILD_TYPE")
if [ -n "${QT_DIR:-}" ]; then
    CMAKE_ARGS+=(-DCMAKE_PREFIX_PATH="$QT_DIR")
fi

echo "→ Configuring Pier-X ($BUILD_TYPE) in $BUILD_DIR"
cmake "${CMAKE_ARGS[@]}"

echo "→ Building"
cmake --build "$BUILD_DIR" --config "$BUILD_TYPE" --parallel

echo "✓ Build complete: $BUILD_DIR"
