# ─────────────────────────────────────────────────────────
# pier-ui-qt — Qt 6 / QML UI shell for Pier-X
# ─────────────────────────────────────────────────────────

find_package(Qt6 6.8 REQUIRED COMPONENTS
    Core
    Gui
    Qml
    Quick
    QuickControls2
    LinguistTools
)

qt_standard_project_setup(REQUIRES 6.8)

qt_add_executable(pier-x
    src/main.cpp
)

# qmltyperegistrar emits `#if __has_include(<PierCoreBridge.h>)` in its
# generated bootstrap, so the source directory has to be on the include
# path even though we don't `#include` anything from it ourselves.
target_include_directories(pier-x PRIVATE src)

# Mark Theme.qml as a QML singleton — must be set BEFORE qt_add_qml_module
set_source_files_properties(qml/Theme.qml
    PROPERTIES QT_QML_SINGLETON_TYPE TRUE
)

qt_add_qml_module(pier-x
    URI Pier
    VERSION 1.0
    SOURCES
        src/PierCoreBridge.h
        src/PierCoreBridge.cpp
        src/PierTerminalSession.h
        src/PierTerminalSession.cpp
        src/PierTerminalGrid.h
        src/PierTerminalGrid.cpp
        src/PierConnectionStore.h
        src/PierConnectionStore.cpp
        src/PierCredentials.h
        src/PierCredentials.cpp
        src/PierSftpBrowser.h
        src/PierSftpBrowser.cpp
    QML_FILES
        qml/Main.qml
        qml/Theme.qml
        qml/components/PrimaryButton.qml
        qml/components/GhostButton.qml
        qml/components/IconButton.qml
        qml/components/PierTextField.qml
        qml/components/PierComboBox.qml
        qml/components/PierToolTip.qml
        qml/components/Card.qml
        qml/components/StatusPill.qml
        qml/components/SectionLabel.qml
        qml/components/Separator.qml
        qml/components/TerminalTab.qml
        qml/shell/TopBar.qml
        qml/shell/TabBar.qml
        qml/shell/Sidebar.qml
        qml/shell/StatusBar.qml
        qml/shell/WelcomeView.qml
        qml/shell/CommandPalette.qml
        qml/dialogs/NewConnectionDialog.qml
        qml/dialogs/SettingsDialog.qml
        qml/views/TerminalView.qml
        qml/views/SftpBrowserView.qml
    RESOURCES
        resources/icons/pier.png
)

# ─── i18n ─────────────────────────────────────────────
# lupdate extracts qsTr() strings → .ts files;
# lrelease compiles them → .qm files embedded in the binary.
qt_add_translations(pier-x
    TS_FILES
        i18n/pier-x_zh_CN.ts
    RESOURCE_PREFIX /i18n
)

target_link_libraries(pier-x PRIVATE
    Qt6::Core
    Qt6::Gui
    Qt6::Qml
    Qt6::Quick
    Qt6::QuickControls2
    pier_core
)

# Rust staticlibs link against the platform's runtime libraries. Corrosion
# tells us which ones on each platform, but when linking a staticlib into
# a Qt executable we need to forward them explicitly.
if(APPLE)
    target_link_libraries(pier-x PRIVATE
        "-framework CoreFoundation"
        "-framework Security"
        "-framework SystemConfiguration"
    )
elseif(WIN32)
    target_link_libraries(pier-x PRIVATE
        ws2_32 userenv advapi32 bcrypt ntdll crypt32 secur32 ncrypt
    )
endif()

target_compile_definitions(pier-x PRIVATE
    PIER_X_VERSION="${PROJECT_VERSION}"
)

set_target_properties(pier-x PROPERTIES
    WIN32_EXECUTABLE ON
    MACOSX_BUNDLE ON
    MACOSX_BUNDLE_BUNDLE_NAME "Pier-X"
    MACOSX_BUNDLE_GUI_IDENTIFIER "com.kkape.pier-x"
    MACOSX_BUNDLE_BUNDLE_VERSION "${PROJECT_VERSION}"
    MACOSX_BUNDLE_SHORT_VERSION_STRING "${PROJECT_VERSION}"
)

install(TARGETS pier-x
    BUNDLE  DESTINATION .
    RUNTIME DESTINATION bin
)
