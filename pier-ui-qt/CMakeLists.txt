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
    Svg
)

qt_standard_project_setup(REQUIRES 6.8)

qt_add_executable(pier-x
    src/main.cpp
)

set(PIER_NATIVE_WINDOW_SOURCES
    src/PierNativeWindow.h
)

if(APPLE)
    list(APPEND PIER_NATIVE_WINDOW_SOURCES
        src/PierNativeWindow_mac.mm
    )
elseif(WIN32)
    list(APPEND PIER_NATIVE_WINDOW_SOURCES
        src/PierNativeWindow_win.cpp
    )
endif()

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
        ${PIER_NATIVE_WINDOW_SOURCES}
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
        src/PierServiceDetector.h
        src/PierServiceDetector.cpp
        src/PierTunnel.h
        src/PierTunnel.cpp
        src/PierRedisClient.h
        src/PierRedisClient.cpp
        src/PierLogStream.h
        src/PierLogStream.cpp
        src/PierDockerClient.h
        src/PierDockerClient.cpp
        src/PierMarkdown.h
        src/PierMarkdown.cpp
        src/PierMySqlClient.h
        src/PierMySqlClient.cpp
        src/PierMySqlWorkspace.h
        src/PierMySqlWorkspace.cpp
        src/PierGitClient.h
        src/PierGitClient.cpp
    QML_FILES
        qml/Main.qml
        qml/Theme.qml
        qml/components/PrimaryButton.qml
        qml/components/GhostButton.qml
        qml/components/IconButton.qml
        qml/components/PierTextField.qml
        qml/components/PierComboBox.qml
        qml/components/SegmentedControl.qml
        qml/components/ToggleSwitch.qml
        qml/components/PierToolTip.qml
        qml/components/Card.qml
        qml/components/StatusPill.qml
        qml/components/SectionLabel.qml
        qml/components/Separator.qml
        qml/components/TerminalTab.qml
        qml/components/ToastManager.qml
        qml/shell/TopBar.qml
        qml/shell/TabBar.qml
        qml/shell/Sidebar.qml
        qml/shell/LocalFilesPane.qml
        qml/shell/StatusBar.qml
        qml/shell/WelcomeView.qml
        qml/shell/CommandPalette.qml
        qml/shell/RightPanel.qml
        qml/shell/RightSidebar.qml
        qml/dialogs/NewConnectionDialog.qml
        qml/dialogs/SettingsDialog.qml
        qml/dialogs/CommandHistoryDialog.qml
        qml/views/TerminalView.qml
        qml/views/SftpBrowserView.qml
        qml/views/RedisBrowserView.qml
        qml/views/MySqlBrowserView.qml
        qml/views/LogViewerView.qml
        qml/views/DockerPanelView.qml
        qml/views/MarkdownPreviewView.qml
        qml/views/GitPanelView.qml
    RESOURCES
        resources/icons/pier.png
        resources/icons/lucide/chevron-down.svg
        resources/icons/lucide/terminal.svg
        resources/icons/lucide/file-text.svg
        resources/icons/lucide/x.svg
        resources/icons/lucide/refresh-cw.svg
        resources/icons/lucide/link.svg
        resources/icons/lucide/settings.svg
        resources/icons/lucide/plus.svg
        resources/icons/lucide/loader.svg
        resources/icons/lucide/sun.svg
        resources/icons/lucide/command.svg
        resources/icons/lucide/arrow-left.svg
        resources/icons/lucide/folder.svg
        resources/icons/lucide/moon.svg
        resources/icons/lucide/git-branch.svg
        resources/icons/lucide/database.svg
        resources/icons/lucide/container.svg
        resources/icons/lucide/file-code.svg
        resources/icons/lucide/server.svg
        resources/icons/lucide/layers.svg
        resources/icons/lucide/search.svg
        resources/fonts/Inter-Medium.ttf
        resources/fonts/JetBrainsMono-Regular.ttf
        resources/fonts/Inter-Regular.ttf
        resources/fonts/JetBrainsMono-Medium.ttf
        resources/fonts/Inter-SemiBold.ttf
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
    Qt6::Svg
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
        iconv
        z
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
