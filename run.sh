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

find_qt6() {
    # 1. Explicit QT_DIR wins
    if [ -n "${QT_DIR:-}" ] && [ -f "$QT_DIR/lib/cmake/Qt6/Qt6Config.cmake" ]; then
        echo "$QT_DIR"
        return 0
    fi

    # 2. qmake / qmake6 already in PATH
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

    # 3. Scan common install roots
    local arch
    case "$(uname -s)" in
        Darwin*)
            # Homebrew first (covers both Intel and Apple Silicon)
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

print_qt_install_help() {
    cat >&2 <<'EOF'

ERROR: Qt 6.8 not found.

Pier-X needs Qt 6.8 LTS (or newer) installed. Pick one:

  Option A - aqtinstall (recommended, matches CI):
    pip install aqtinstall
EOF
    case "$(uname -s)" in
        Darwin*)
            cat >&2 <<'EOF'
    aqt install-qt mac desktop 6.8.1 clang_64 --outputdir ~/Qt
    QT_DIR=~/Qt/6.8.1/macos ./run.sh
EOF
            ;;
        *)
            cat >&2 <<'EOF'
    aqt install-qt linux desktop 6.8.1 linux_gcc_64 --outputdir ~/Qt
    QT_DIR=~/Qt/6.8.1/gcc_64 ./run.sh
EOF
            ;;
    esac
    case "$(uname -s)" in
        Darwin*)
            cat >&2 <<'EOF'

  Option B - Homebrew:
    brew install qt
    ./run.sh

EOF
            ;;
        Linux*)
            cat >&2 <<'EOF'

  Option B - Distro package (Debian / Ubuntu):
    sudo apt install qt6-base-dev qt6-declarative-dev qt6-shadertools-dev
    ./run.sh

EOF
            ;;
    esac
    cat >&2 <<'EOF'
  Option C - if Qt is already installed somewhere unusual:
    QT_DIR=/path/containing/lib/cmake/Qt6 ./run.sh

EOF
}

BUILD_TYPE="${BUILD_TYPE:-Release}"
BUILD_DIR="${BUILD_DIR:-build}"

QT_PREFIX="$(find_qt6)" || {
    print_qt_install_help
    exit 1
}

echo "==> Found Qt at: $QT_PREFIX"

CMAKE_ARGS=(-B "$BUILD_DIR" -S . -DCMAKE_BUILD_TYPE="$BUILD_TYPE" -DCMAKE_PREFIX_PATH="$QT_PREFIX")

echo "==> Configuring Pier-X ($BUILD_TYPE) in $BUILD_DIR"
cmake "${CMAKE_ARGS[@]}"

echo "==> Building"
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
    echo "ERROR: $APP not found or not executable" >&2
    exit 1
fi

echo "==> Launching $APP"
exec "$APP" "$@"
