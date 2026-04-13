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

set(PIER_PLATFORM_APP_ICON_SOURCES)

if(WIN32)
    list(APPEND PIER_PLATFORM_APP_ICON_SOURCES
        resources/icons/pier-windows.rc
    )
endif()

qt_add_executable(pier-x
    src/main.cpp
    ${PIER_PLATFORM_APP_ICON_SOURCES}
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

# ─── Auto-update (platform-conditional) ───────────────
set(PIER_UPDATER_SOURCES
    src/PierUpdater.h
)

if(APPLE)
    list(APPEND PIER_UPDATER_SOURCES
        src/PierUpdater_mac.mm
    )
elseif(WIN32)
    list(APPEND PIER_UPDATER_SOURCES
        src/PierUpdater_win.cpp
    )
else()
    list(APPEND PIER_UPDATER_SOURCES
        src/PierUpdater_stub.cpp
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

set(PIER_QML_FILES
    qml/Main.qml
    qml/Theme.qml
    qml/components/PrimaryButton.qml
    qml/components/GhostButton.qml
    qml/components/IconButton.qml
    qml/components/PierTextField.qml
    qml/components/PierTextArea.qml
    qml/components/PierSearchField.qml
    qml/components/PierComboBox.qml
    qml/components/PierSplitHandle.qml
    qml/components/PierScrollView.qml
    qml/components/PierScrollBar.qml
    qml/components/PierMenuItem.qml
    qml/components/PierSlider.qml
    qml/components/SegmentedControl.qml
    qml/components/ToggleSwitch.qml
    qml/components/PierToolTip.qml
    qml/components/Card.qml
    qml/components/ToolPanelSurface.qml
    qml/components/ToolHeroPanel.qml
    qml/components/ToolSectionHeader.qml
    qml/components/ToolEmptyState.qml
    qml/components/ToolBanner.qml
    qml/components/ToolFactChip.qml
    qml/components/ToolMetricTile.qml
    qml/components/StatusPill.qml
    qml/components/ModalDialogShell.qml
    qml/components/PopoverPanel.qml
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
    qml/views/PostgresPanelView.qml
    qml/views/LogViewerView.qml
    qml/views/DockerPanelView.qml
    qml/views/MarkdownPreviewView.qml
    qml/views/GitPanelView.qml
    qml/views/ServerMonitorView.qml
    qml/views/SqliteBrowserView.qml
)

set(PIER_QML_RESOURCES
    resources/icons/pier.png
    resources/icons/pier-windows.png
    resources/icons/lucide/chevron-down.svg
    resources/icons/lucide/terminal.svg
    resources/icons/lucide/file-text.svg
    resources/icons/lucide/x.svg
    resources/icons/lucide/refresh-cw.svg
    resources/icons/lucide/link.svg
    resources/icons/lucide/settings.svg
    resources/icons/lucide/plus.svg
    resources/icons/lucide/loader.svg
    resources/icons/lucide/activity.svg
    resources/icons/lucide/bolt.svg
    resources/icons/lucide/scroll-text.svg
    resources/icons/lucide/sun.svg
    resources/icons/lucide/command.svg
    resources/icons/lucide/arrow-left.svg
    resources/icons/lucide/folder.svg
    resources/icons/lucide/folder-sync.svg
    resources/icons/lucide/moon.svg
    resources/icons/lucide/git-branch.svg
    resources/icons/lucide/database.svg
    resources/icons/lucide/container.svg
    resources/icons/lucide/file-code.svg
    resources/icons/lucide/server.svg
    resources/icons/lucide/layers.svg
    resources/icons/lucide/search.svg
    resources/icons/lucide/hard-drive.svg
    resources/fonts/Inter-Medium.ttf
    resources/fonts/JetBrainsMono-Regular.ttf
    resources/fonts/Inter-Regular.ttf
    resources/fonts/JetBrainsMono-Medium.ttf
    resources/fonts/Inter-SemiBold.ttf
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
        src/PierLocalSystem.h
        src/PierLocalSystem.cpp
        src/PierSshSessionHandle.h
        src/PierSshSessionHandle.cpp
        src/PierControlMasterHandle.h
        src/PierControlMasterHandle.cpp
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
        src/PierPostgresClient.h
        src/PierPostgresClient.cpp
        src/PierGitClient.h
        src/PierGitClient.cpp
        src/PierSqliteClient.h
        src/PierSqliteClient.cpp
        src/PierSearch.h
        src/PierSearch.cpp
        src/PierServerMonitor.h
        src/PierServerMonitor.cpp
        src/PierI18n.h
        src/PierI18n.cpp
        src/PierProfiler.h
        src/PierProfiler.cpp
        src/PierWindowChrome.h
        src/PierWindowChrome.cpp
        ${PIER_UPDATER_SOURCES}
        src/PierUpdaterBridge.h
        src/PierUpdaterBridge.cpp
    QML_FILES
        ${PIER_QML_FILES}
    RESOURCES
        ${PIER_QML_RESOURCES}
)

# ─── i18n ─────────────────────────────────────────────
# lupdate extracts qsTr() strings → .ts files;
# lrelease compiles them → .qm files embedded in the binary.
qt_add_translations(pier-x
    TS_FILES
        i18n/pier-x_zh_CN.ts
    SOURCES
        ${PIER_QML_FILES}
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
    # ── Sparkle auto-update framework ──────────────────
    # Look for a local copy first (thirdparty/ or system),
    # fall back gracefully if not found — the stub compiles
    # but PierUpdater::available() returns false at runtime.
    find_library(SPARKLE_FRAMEWORK Sparkle
        PATHS ${CMAKE_CURRENT_SOURCE_DIR}/thirdparty/sparkle
              /Library/Frameworks
              ~/Library/Frameworks
    )
    if(SPARKLE_FRAMEWORK)
        target_link_libraries(pier-x PRIVATE ${SPARKLE_FRAMEWORK})
        target_compile_definitions(pier-x PRIVATE PIER_HAS_SPARKLE=1)
        # Ensure the framework is found at runtime inside the bundle.
        set_target_properties(pier-x PROPERTIES
            BUILD_WITH_INSTALL_RPATH TRUE
            INSTALL_RPATH "@loader_path/../Frameworks"
        )
    else()
        message(STATUS "Sparkle.framework not found — auto-update disabled")
    endif()

    target_link_libraries(pier-x PRIVATE
        "-framework CoreFoundation"
        "-framework Security"
        "-framework SystemConfiguration"
        iconv
        z
    )
elseif(WIN32)
    # ── WinSparkle auto-update library ─────────────────
    if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/thirdparty/winsparkle/include/winsparkle.h")
        target_include_directories(pier-x PRIVATE
            ${CMAKE_CURRENT_SOURCE_DIR}/thirdparty/winsparkle/include)
        target_link_libraries(pier-x PRIVATE
            ${CMAKE_CURRENT_SOURCE_DIR}/thirdparty/winsparkle/lib/WinSparkle.lib)
        target_compile_definitions(pier-x PRIVATE PIER_HAS_WINSPARKLE=1)
    else()
        message(STATUS "WinSparkle not found — auto-update disabled")
    endif()

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

if(WIN32)
    get_target_property(QT_QMAKE_EXECUTABLE Qt6::qmake IMPORTED_LOCATION)
    get_filename_component(QT_BIN_DIR "${QT_QMAKE_EXECUTABLE}" DIRECTORY)
    find_program(WINDEPLOYQT_EXECUTABLE windeployqt HINTS "${QT_BIN_DIR}" REQUIRED)

    add_custom_command(TARGET pier-x POST_BUILD
        COMMAND "${WINDEPLOYQT_EXECUTABLE}"
            "$<$<CONFIG:Debug>:--debug>"
            "$<$<NOT:$<CONFIG:Debug>>:--release>"
            --qmldir "${CMAKE_CURRENT_SOURCE_DIR}/qml"
            --dir "$<TARGET_FILE_DIR:pier-x>"
            "$<TARGET_FILE:pier-x>"
        COMMENT "Deploying Qt runtime for $<CONFIG> build"
        COMMAND_EXPAND_LISTS
    )
endif()
