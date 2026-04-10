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
# Subprojects
# ─────────────────────────────────────────────────────────

# pier-core: Rust core engine — wired in once the cxx-qt bridge lands.
# add_subdirectory(pier-core)

# pier-ui-qt: Qt 6 / QML UI shell
add_subdirectory(pier-ui-qt)
