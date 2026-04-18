#!/usr/bin/env bash
# package-linux.sh — build Linux release artifacts for Pier-X.
#
# Usage:
#   PACKAGE_FORMATS=deb,appimage ./scripts/package-linux.sh
#   BUILD_TYPE=Release PACKAGE_FORMATS=deb ./scripts/package-linux.sh --skip-build

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
UI_CRATE="${PIER_UI_CRATE:-pier-ui-gpui}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
BUILD_DIR="${BUILD_DIR:-}"
PACKAGE_FORMATS="${PACKAGE_FORMATS:-deb,appimage}"
SKIP_BUILD=0

for arg in "$@"; do
    case "$arg" in
        --skip-build)
            SKIP_BUILD=1
            ;;
        *)
            echo "ERROR: unknown argument: $arg" >&2
            exit 1
            ;;
    esac
done

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "ERROR: required command not found: $1" >&2
        exit 1
    fi
}

ensure_ui_dir() {
    if [ ! -d "$ROOT_DIR/$UI_CRATE" ]; then
        echo "ERROR: active GPUI shell crate not found at $ROOT_DIR/$UI_CRATE" >&2
        exit 1
    fi
}

normalized_formats() {
    printf '%s' "$PACKAGE_FORMATS" | tr '[:upper:]' '[:lower:]' | tr '; ' ',,'
}

format_requested() {
    local wanted=",$(normalized_formats),"
    case "$wanted" in
        *,all,*|*,"$1",*)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

resolve_target_root() {
    if [ -n "${CARGO_TARGET_DIR:-}" ]; then
        printf '%s\n' "$CARGO_TARGET_DIR"
    else
        printf '%s\n' "$ROOT_DIR/target"
    fi
}

resolve_package_root() {
    local requested="${PACKAGE_OUTPUT_DIR:-}"
    if [ -z "$requested" ]; then
        printf '%s\n' "$TARGET_ROOT/$PROFILE_DIR/packages"
        return 0
    fi

    case "$requested" in
        /*)
            printf '%s\n' "$requested"
            ;;
        *)
            printf '%s\n' "$ROOT_DIR/$requested"
            ;;
    esac
}

resolve_version() {
    sed -n 's/^version *= *"\(.*\)"$/\1/p' "$ROOT_DIR/pier-ui-gpui/Cargo.toml" | head -n 1
}

write_desktop_file() {
    local path="$1"
    local exec_cmd="$2"
    cat > "$path" <<EOF
[Desktop Entry]
Version=1.0
Type=Application
Name=Pier-X
Comment=Cross-platform terminal management on GPUI + Rust core
Exec=$exec_cmd
Icon=pier-x
Terminal=false
Categories=Development;System;Utility;
StartupWMClass=Pier-X
EOF
}

prepare_build() {
    if [ "$SKIP_BUILD" -eq 1 ]; then
        return 0
    fi

    local cargo_cmd=(cargo build -p "$UI_CRATE")
    if [ "$BUILD_TYPE" = "Release" ]; then
        cargo_cmd+=(--release)
    fi

    echo "==> Building Pier-X Linux payload ($BUILD_TYPE)"
    ( cd "$ROOT_DIR" && "${cargo_cmd[@]}" )
}

package_deb() {
    need_cmd dpkg-deb

    local stage_dir="$LINUX_PACKAGE_ROOT/stage/deb/pier-x_${PACKAGE_VERSION_DEB}_${DEB_ARCH}"
    local deb_path="$LINUX_PACKAGE_ROOT/Pier-X_${PACKAGE_VERSION_DEB}_linux_${DEB_ARCH}.deb"

    echo "==> Packaging .deb -> $deb_path"
    rm -rf "$stage_dir"
    mkdir -p \
        "$stage_dir/DEBIAN" \
        "$stage_dir/opt/Pier-X" \
        "$stage_dir/usr/bin" \
        "$stage_dir/usr/share/applications" \
        "$stage_dir/usr/share/icons/hicolor/256x256/apps" \
        "$stage_dir/usr/share/icons/hicolor/512x512/apps"

    cp "$TARGET_BIN" "$stage_dir/opt/Pier-X/Pier-X"
    chmod 755 "$stage_dir/opt/Pier-X/Pier-X"
    cp "$ROOT_DIR/README.md" "$stage_dir/opt/Pier-X/README.md"
    cp "$ROOT_DIR/LICENSE" "$stage_dir/opt/Pier-X/LICENSE"
    cp "$ICON_SRC" "$stage_dir/usr/share/icons/hicolor/256x256/apps/pier-x.png"
    cp "$ICON_SRC" "$stage_dir/usr/share/icons/hicolor/512x512/apps/pier-x.png"
    ln -s "/opt/Pier-X/Pier-X" "$stage_dir/usr/bin/pier-x"
    write_desktop_file "$stage_dir/usr/share/applications/pier-x.desktop" "/opt/Pier-X/Pier-X"

    cat > "$stage_dir/DEBIAN/control" <<EOF
Package: pier-x
Version: $PACKAGE_VERSION_DEB
Section: utils
Priority: optional
Architecture: $DEB_ARCH
Maintainer: Pier-X
Description: Cross-platform terminal management on GPUI + Rust core
EOF

    mkdir -p "$(dirname "$deb_path")"
    rm -f "$deb_path"
    if dpkg-deb --help 2>/dev/null | grep -q -- '--root-owner-group'; then
        dpkg-deb --build --root-owner-group "$stage_dir" "$deb_path"
    else
        dpkg-deb --build "$stage_dir" "$deb_path"
    fi
    echo "[OK] Linux package ready: $deb_path"
}

package_appimage() {
    need_cmd appimagetool

    local appdir="$LINUX_PACKAGE_ROOT/stage/appimage/Pier-X.AppDir"
    local appimage_path="$LINUX_PACKAGE_ROOT/Pier-X_${PACKAGE_VERSION}_linux_${APPIMAGE_ARCH}.AppImage"
    local desktop_path="$appdir/usr/share/applications/pier-x.desktop"

    echo "==> Packaging AppImage -> $appimage_path"
    rm -rf "$appdir"
    mkdir -p \
        "$appdir/usr/bin" \
        "$appdir/usr/share/doc/pier-x" \
        "$appdir/usr/share/applications" \
        "$appdir/usr/share/icons/hicolor/512x512/apps"

    cp "$TARGET_BIN" "$appdir/usr/bin/Pier-X"
    chmod 755 "$appdir/usr/bin/Pier-X"
    cp "$ROOT_DIR/README.md" "$appdir/usr/share/doc/pier-x/README.md"
    cp "$ROOT_DIR/LICENSE" "$appdir/usr/share/doc/pier-x/LICENSE"
    cp "$ICON_SRC" "$appdir/pier-x.png"
    cp "$ICON_SRC" "$appdir/usr/share/icons/hicolor/512x512/apps/pier-x.png"
    ln -sf "pier-x.png" "$appdir/.DirIcon"
    write_desktop_file "$desktop_path" "Pier-X"
    cp "$desktop_path" "$appdir/pier-x.desktop"

    cat > "$appdir/AppRun" <<'EOF'
#!/usr/bin/env bash
HERE="$(cd "$(dirname "$0")" && pwd)"
exec "$HERE/usr/bin/Pier-X" "$@"
EOF
    chmod 755 "$appdir/AppRun"

    if command -v linuxdeploy >/dev/null 2>&1; then
        echo "==> Running linuxdeploy to bundle shared-library dependencies"
        linuxdeploy \
            --appdir "$appdir" \
            --executable "$appdir/usr/bin/Pier-X" \
            --desktop-file "$desktop_path" \
            --icon-file "$ICON_SRC" >/dev/null
        chmod 755 "$appdir/AppRun"
    else
        echo "WARN: linuxdeploy not found; AppImage will be built without dependency scanning" >&2
    fi

    mkdir -p "$(dirname "$appimage_path")"
    rm -f "$appimage_path"
    ARCH="$APPIMAGE_ARCH" appimagetool "$appdir" "$appimage_path"
    chmod 755 "$appimage_path"
    echo "[OK] Linux package ready: $appimage_path"
}

case "$BUILD_TYPE" in
    Debug)
        PROFILE_DIR="debug"
        ;;
    Release)
        PROFILE_DIR="release"
        ;;
    *)
        echo "ERROR: BUILD_TYPE must be Debug or Release (got: $BUILD_TYPE)" >&2
        exit 1
        ;;
esac

ensure_ui_dir
need_cmd cargo

if [ -n "$BUILD_DIR" ] && [ -z "${CARGO_TARGET_DIR:-}" ]; then
    case "$BUILD_DIR" in
        /*)
            export CARGO_TARGET_DIR="$BUILD_DIR"
            ;;
        *)
            export CARGO_TARGET_DIR="$ROOT_DIR/$BUILD_DIR"
            ;;
    esac
fi

TARGET_ROOT="$(resolve_target_root)"
TARGET_BIN="$TARGET_ROOT/$PROFILE_DIR/$UI_CRATE"
PACKAGE_ROOT="$(resolve_package_root)"
LINUX_PACKAGE_ROOT="$PACKAGE_ROOT/linux"
ICON_SRC="$ROOT_DIR/pier-ui-gpui/assets/app-icons/icon.png"
PACKAGE_VERSION="$(resolve_version)"
PACKAGE_VERSION_DEB="$(printf '%s' "$PACKAGE_VERSION" | sed 's/-/~/g')"

case "$(uname -m)" in
    x86_64|amd64)
        DEB_ARCH="amd64"
        APPIMAGE_ARCH="x86_64"
        ;;
    aarch64|arm64)
        DEB_ARCH="arm64"
        APPIMAGE_ARCH="aarch64"
        ;;
    *)
        echo "ERROR: unsupported Linux architecture for packaging: $(uname -m)" >&2
        exit 1
        ;;
esac

prepare_build

if [ ! -f "$TARGET_BIN" ]; then
    echo "ERROR: binary not found at $TARGET_BIN" >&2
    exit 1
fi

if [ ! -f "$ICON_SRC" ]; then
    echo "ERROR: icon not found at $ICON_SRC" >&2
    exit 1
fi

mkdir -p "$LINUX_PACKAGE_ROOT"

PACKAGED_ANY=0

if format_requested "deb"; then
    PACKAGED_ANY=1
    package_deb
fi

if format_requested "appimage"; then
    PACKAGED_ANY=1
    package_appimage
fi

if [ "$PACKAGED_ANY" -eq 0 ]; then
    echo "ERROR: unsupported Linux PACKAGE_FORMATS=$PACKAGE_FORMATS (supported: deb, appimage, all)" >&2
    exit 1
fi
