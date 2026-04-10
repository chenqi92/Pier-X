import QtQuick
import QtQuick.Window
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Pier

ApplicationWindow {
    id: window
    width: 1280
    height: 800
    minimumWidth: 800
    minimumHeight: 500
    visible: true
    title: qsTr("Pier-X")

    color: Theme.bgCanvas
    Behavior on color {
        ColorAnimation { duration: Theme.durNormal; easing.type: Theme.easingType }
    }

    // ─────────────────────────────────────────────────────
    // App-wide tab model — each entry is one terminal session.
    // Will become a C++/Rust model once pier-core lands; for now
    // it's a plain QML ListModel so the UI can be exercised.
    // ─────────────────────────────────────────────────────
    ListModel {
        id: tabModel
    }

    property int currentTabIndex: 0

    function openNewTab() {
        const n = tabModel.count + 1
        tabModel.append({ title: qsTr("Local %1").arg(n) })
        currentTabIndex = tabModel.count - 1
    }

    function closeTab(index) {
        if (index < 0 || index >= tabModel.count)
            return
        tabModel.remove(index)
        if (currentTabIndex >= tabModel.count) {
            currentTabIndex = Math.max(0, tabModel.count - 1)
        }
    }

    // ─────────────────────────────────────────────────────
    // IDE shell
    //   ┌──────────────────────────────────────┐
    //   │              TopBar                  │
    //   ├─────────┬────────────────────────────┤
    //   │         │  TabBar (when tabs > 0)    │
    //   │ Sidebar │ ───────────────────────────│
    //   │         │  TerminalView | Welcome    │
    //   ├─────────┴────────────────────────────┤
    //   │             StatusBar                │
    //   └──────────────────────────────────────┘
    // ─────────────────────────────────────────────────────
    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        TopBar {
            Layout.fillWidth: true
            onNewSessionRequested: window.openNewTab()
            onCommandPaletteRequested: console.log("Command palette — TODO")
            onSettingsRequested: console.log("Settings — TODO")
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            Sidebar {
                Layout.fillHeight: true
            }

            // Main content area
            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                // Empty state
                WelcomeView {
                    anchors.fill: parent
                    visible: tabModel.count === 0
                }

                // Tab area (visible when at least one tab exists)
                ColumnLayout {
                    anchors.fill: parent
                    visible: tabModel.count > 0
                    spacing: 0

                    TabBar {
                        Layout.fillWidth: true
                        model: tabModel
                        currentIndex: window.currentTabIndex
                        onTabClicked: (i) => window.currentTabIndex = i
                        onTabClosed: (i) => window.closeTab(i)
                        onNewTabClicked: window.openNewTab()
                    }

                    // Content swap based on currentTabIndex
                    Loader {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        active: tabModel.count > 0 && window.currentTabIndex < tabModel.count
                        sourceComponent: TerminalView {
                            title: tabModel.count > 0 && window.currentTabIndex < tabModel.count
                                 ? tabModel.get(window.currentTabIndex).title
                                 : ""
                        }
                    }
                }
            }
        }

        StatusBar {
            Layout.fillWidth: true
        }
    }
}
