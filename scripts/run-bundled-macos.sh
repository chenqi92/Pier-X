#!/usr/bin/env bash
# run-bundled-macos.sh — convenience wrapper: build → bundle → open the .app.
#
# Use this instead of `./run.sh` when you want the proper Pier-X dock icon
# (the bare cargo binary inherits the parent terminal's icon on macOS).
#
# Usage:
#   ./scripts/run-bundled-macos.sh
#   BUILD_TYPE=Release ./scripts/run-bundled-macos.sh

set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
exec "$ROOT_DIR/scripts/bundle-macos.sh" --open
