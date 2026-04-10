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

find_qt6() {
    if [ -n "${QT_DIR:-}" ] && [ -f "$QT_DIR/lib/cmake/Qt6/Qt6Config.cmake" ]; then
        echo "$QT_DIR"
        return 0
    fi
    for name in qmake6 qmake; do
        if command -v "$name" >/dev/null 2>&1; then
            local prefix
            prefix="$("$name" -query QT_INSTALL_PREFIX 2>/dev/null || true)"
            if [ -n "$prefix" ] && [ -f "$prefix/lib/cmake/Qt6/Qt6Config.cmake" ]; then
                echo "$prefix"
                return 0
            fi
        fi
    done
    local arch
    case "$(uname -s)" in
        Darwin*)
            for direct in "/opt/homebrew/opt/qt" "/usr/local/opt/qt" "/opt/homebrew/opt/qt6" "/usr/local/opt/qt6"; do
                if [ -f "$direct/lib/cmake/Qt6/Qt6Config.cmake" ]; then
                    echo "$direct"
                    return 0
                fi
            done
            arch="macos"
            ;;
        *)
            arch="gcc_64"
            ;;
    esac
    for root in "$HOME/Qt" "/opt/Qt"; do
        [ -d "$root" ] || continue
        local vd
        for vd in $(ls -1 "$root" 2>/dev/null | grep -E '^6\.[0-9]+(\.[0-9]+)?$' | sort -rV); do
            local cand="$root/$vd/$arch"
            if [ -f "$cand/lib/cmake/Qt6/Qt6Config.cmake" ]; then
                echo "$cand"
                return 0
            fi
        done
    done
    return 1
}

BUILD_TYPE="${BUILD_TYPE:-Release}"
BUILD_DIR="${BUILD_DIR:-build}"

QT_PREFIX="$(find_qt6)" || {
    echo "" >&2
    echo "ERROR: Qt 6.8 not found." >&2
    echo "Install Qt 6.8 LTS, then re-run this script." >&2
    echo "Easiest path: pip install aqtinstall && aqt install-qt ..." >&2
    echo "Or set QT_DIR explicitly if Qt is installed elsewhere." >&2
    echo "" >&2
    exit 1
}

echo "==> Found Qt at: $QT_PREFIX"

CMAKE_ARGS=(-B "$BUILD_DIR" -S . -DCMAKE_BUILD_TYPE="$BUILD_TYPE" -DCMAKE_PREFIX_PATH="$QT_PREFIX")

echo "==> Configuring Pier-X ($BUILD_TYPE) in $BUILD_DIR"
cmake "${CMAKE_ARGS[@]}"

echo "==> Building"
cmake --build "$BUILD_DIR" --config "$BUILD_TYPE" --parallel

echo "[OK] Build complete: $BUILD_DIR"
