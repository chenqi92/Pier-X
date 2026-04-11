# ─────────────────────────────────────────────────────────
# pier-core — cross-platform Rust core engine
# ─────────────────────────────────────────────────────────
#
# This file hands pier-core off to Corrosion, which drives `cargo build`
# and exposes the resulting static library as the CMake target
# `pier_core-static`. Callers should link against `pier_core` (an alias
# created below) which also pulls in the C-ABI header search path.
#
# The crate itself lives in src/ and is built with plain cargo for
# everything else (tests, clippy, docs, editor integration). CMake is
# only responsible for (1) building the right profile and (2) handing
# the staticlib + OS system libs to pier-ui-qt at link time.

corrosion_import_crate(
    MANIFEST_PATH ${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml
    CRATES        pier-core
)

# Give the imported staticlib target a public include directory so callers
# that consume the C ABI can `#include "pier_core.h"` without knowing
# about cbindgen-generated header plumbing. We ship a hand-written header
# today; a cbindgen step will replace this once the FFI surface grows
# beyond a handful of functions.
target_include_directories(pier_core-static
    INTERFACE
    ${CMAKE_CURRENT_SOURCE_DIR}/include
)

# Stable alias so consumers can spell it without the `-static` suffix.
# (`corrosion_import_crate` produces `<crate>-static` for staticlibs.)
if(NOT TARGET pier_core)
    add_library(pier_core INTERFACE)
    target_link_libraries(pier_core INTERFACE pier_core-static)
endif()
