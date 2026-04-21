#!/usr/bin/env bash
# run.sh — Launch the active Tauri shell from the repo root.
#
# Usage:
#   ./run.sh                        # Debug/dev shell
#   BUILD_TYPE=Release ./run.sh     # Run the Tauri dev shell in release mode
#   BUILD_DIR=target-root ./run.sh  # Override Cargo target dir

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
UI_DIR="${PIER_UI_DIR:-$ROOT_DIR/pier-ui-tauri}"
BUILD_TYPE="${BUILD_TYPE:-Debug}"
BUILD_DIR="${BUILD_DIR:-}"

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "ERROR: required command not found: $1" >&2
        exit 1
    fi
}

ensure_ui_dir() {
    if [ ! -d "$UI_DIR" ]; then
        echo "ERROR: active Tauri shell not found at $UI_DIR" >&2
        exit 1
    fi
}

ensure_node_modules() {
    local lock_marker="node_modules/.package-lock.json"
    if [ ! -d node_modules ] || [ package-lock.json -nt "$lock_marker" ]; then
        echo "==> Installing frontend dependencies"
        npm ci
    fi
}

case "$BUILD_TYPE" in
    Debug|Release) ;;
    *)
        echo "ERROR: BUILD_TYPE must be Debug or Release (got: $BUILD_TYPE)" >&2
        exit 1
        ;;
esac

need_cmd node
need_cmd npm
need_cmd cargo
ensure_ui_dir

if [ -n "$BUILD_DIR" ]; then
    export CARGO_TARGET_DIR="$ROOT_DIR/$BUILD_DIR"
    echo "==> Using Cargo target dir: $CARGO_TARGET_DIR"
fi

cd "$UI_DIR"
ensure_node_modules

PORT_LINES="$(node "$UI_DIR/scripts/resolve-dev-port.mjs")"
while IFS='=' read -r key value; do
    if [ -n "$key" ] && [ -n "$value" ]; then
        export "$key=$value"
    fi
done <<< "$PORT_LINES"

if [ -z "${PIER_DEV_URL:-}" ] || [ -z "${PIER_DEV_PORT:-}" ]; then
    echo "ERROR: failed to resolve a Tauri dev server port" >&2
    exit 1
fi

TAURI_DEV_CONFIG="$(mktemp "${TMPDIR:-/tmp}/pier-tauri-dev.XXXXXX.json")"
trap 'rm -f "$TAURI_DEV_CONFIG"' EXIT
printf '{\n  "build": {\n    "devUrl": "%s"\n  }\n}\n' "$PIER_DEV_URL" > "$TAURI_DEV_CONFIG"

TAURI_CMD=(npm run tauri -- dev --config "$TAURI_DEV_CONFIG")
if [ "$BUILD_TYPE" = "Release" ]; then
    TAURI_CMD+=(--release)
fi
if [ "$#" -gt 0 ]; then
    TAURI_CMD+=(--)
    TAURI_CMD+=("$@")
fi

echo "==> Launching Pier-X Tauri shell ($BUILD_TYPE)"
echo "==> Using Vite dev server: $PIER_DEV_URL"
"${TAURI_CMD[@]}"
