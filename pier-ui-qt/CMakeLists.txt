# ─────────────────────────────────────────────────────────
# pier-ui-qt — Qt 6 / QML UI shell for Pier-X
# ─────────────────────────────────────────────────────────

find_package(Qt6 6.8 REQUIRED COMPONENTS
    Core
    Gui
    Qml
    Quick
    QuickControls2
)

qt_standard_project_setup(REQUIRES 6.8)

qt_add_executable(pier-x
    src/main.cpp
)

# Mark Theme.qml as a QML singleton — must be set BEFORE qt_add_qml_module
set_source_files_properties(qml/Theme.qml
    PROPERTIES QT_QML_SINGLETON_TYPE TRUE
)

qt_add_qml_module(pier-x
    URI Pier
    VERSION 1.0
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
        qml/views/TerminalView.qml
    RESOURCES
        resources/icons/pier.png
)

target_link_libraries(pier-x PRIVATE
    Qt6::Core
    Qt6::Gui
    Qt6::Qml
    Qt6::Quick
    Qt6::QuickControls2
)

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
