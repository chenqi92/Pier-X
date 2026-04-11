cmake_minimum_required(VERSION 3.21)

# Read version from VERSION file
file(READ "${CMAKE_CURRENT_SOURCE_DIR}/VERSION" PIER_X_VERSION_RAW)
string(STRIP "${PIER_X_VERSION_RAW}" PIER_X_VERSION)

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
