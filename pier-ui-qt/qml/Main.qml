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
    // IDE shell layout
    //   ┌──────────────────────────────────────┐
    //   │              TopBar                  │
    //   ├─────────┬────────────────────────────┤
    //   │         │                            │
    //   │ Sidebar │       WelcomeView          │
    //   │         │                            │
    //   ├─────────┴────────────────────────────┤
    //   │              StatusBar               │
    //   └──────────────────────────────────────┘
    // ─────────────────────────────────────────────────────
    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        TopBar {
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            Sidebar {
                Layout.fillHeight: true
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                WelcomeView {
                    anchors.fill: parent
                }
            }
        }

        StatusBar {
            Layout.fillWidth: true
        }
    }
}
