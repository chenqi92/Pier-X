cmake_minimum_required(VERSION 3.21)

# Read version from VERSION file
file(READ "${CMAKE_CURRENT_SOURCE_DIR}/VERSION" PIER_X_VERSION_RAW)
string(STRIP "${PIER_X_VERSION_RAW}" PIER_X_VERSION)

# ─────────────────────────────────────────────────────────
# macOS deployment target
# ─────────────────────────────────────────────────────────
# Align Qt's deployment target with whatever SDK cargo (and
# therefore the aws-lc-rs assembly objects pulled in by russh)
# targeted. Without this, the Rust assembly objects end up with a
# higher platform version than Qt's C++ compilation units, and
# every link of pier-x prints hundreds of lines like:
#
#   ld: warning: object file (libpier_core.a[aes-xts-dec.o]) was
#       built for newer 'macOS' version (26.4) than being linked (26.0)
#
# These are cosmetic — the binary works fine — but the noise buries
# real link errors if they ever land. The user can override via
# `-DCMAKE_OSX_DEPLOYMENT_TARGET=14.0` (or any other value) to
# deliberately widen the target.
if(APPLE AND NOT CMAKE_OSX_DEPLOYMENT_TARGET)
    execute_process(
        COMMAND xcrun --show-sdk-version
        OUTPUT_VARIABLE _pier_host_sdk
        OUTPUT_STRIP_TRAILING_WHITESPACE
        ERROR_QUIET
    )
    if(_pier_host_sdk)
        set(CMAKE_OSX_DEPLOYMENT_TARGET "${_pier_host_sdk}" CACHE STRING
            "Minimum macOS version (defaults to host SDK)" FORCE)
    endif()
endif()

project(pier-x
    VERSION ${PIER_X_VERSION}
    DESCRIPTION "Cross-platform terminal management built on Qt 6 + Rust"
    HOMEPAGE_URL "https://github.com/chenqi92/Pier-X"
    LANGUAGES CXX
)

# ─────────────────────────────────────────────────────────
# Global build settings
# ─────────────────────────────────────────────────────────
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)

if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
    set(CMAKE_BUILD_TYPE Release CACHE STRING "Build type" FORCE)
endif()

# ─────────────────────────────────────────────────────────
# Rust ↔ CMake integration (Corrosion)
# ─────────────────────────────────────────────────────────
# Corrosion drives cargo from CMake so the Rust core is built as part of
# the normal `cmake --build` flow on every platform. It's the standard
# approach for cargo-in-CMake and is a prerequisite for cxx-qt later —
# once M2 needs QObjects with signals/slots, cxx-qt layers on top of the
# exact same Corrosion-imported crate with no CMake changes here.

include(FetchContent)

FetchContent_Declare(
    Corrosion
    GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
    GIT_TAG        v0.5.2
)
FetchContent_MakeAvailable(Corrosion)

# ─────────────────────────────────────────────────────────
# Subprojects
# ─────────────────────────────────────────────────────────

# pier-core: Rust core engine (static lib, C ABI)
add_subdirectory(pier-core)

# pier-ui-qt: Qt 6 / QML UI shell
add_subdirectory(pier-ui-qt)
