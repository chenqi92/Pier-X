#!/usr/bin/env bash
# run.sh — Configure, build, and launch Pier-X on Unix / macOS.
#
# Usage:
#   ./run.sh                                  # Release build, run
#   BUILD_TYPE=Debug ./run.sh                 # Debug build
#   QT_DIR=~/Qt/6.8.1/macos ./run.sh          # Use a specific Qt install
#   BUILD_DIR=build-debug ./run.sh            # Custom build directory

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

# Locate the binary based on platform
case "$(uname -s)" in
    Darwin*)
        APP="$BUILD_DIR/pier-ui-qt/pier-x.app/Contents/MacOS/pier-x"
        ;;
    *)
        APP="$BUILD_DIR/pier-ui-qt/pier-x"
        ;;
esac

if [ ! -x "$APP" ]; then
    echo "Error: $APP not found or not executable" >&2
    exit 1
fi

echo "→ Launching $APP"
exec "$APP" "$@"
