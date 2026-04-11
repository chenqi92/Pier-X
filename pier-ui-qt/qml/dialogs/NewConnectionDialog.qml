import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier

// Modal dialog — collect SSH connection details and emit `saved` with the
// resulting object. Pure UI; persistence comes from pier-core later.
Item {
    id: root

    property bool open: false
    signal saved(var connection)
    signal cancelled

    visible: open
    z: 9500
    anchors.fill: parent

    function show() {
        nameField.text = ""
        hostField.text = ""
        portField.text = "22"
        userField.text = ""
        passwordField.text = ""
        authCombo.currentIndex = 0
        open = true
        nameField.forceActiveFocus()
    }

    function hide() {
        open = false
    }

    Keys.onEscapePressed: {
        cancelled()
        hide()
    }

    // Backdrop
    Rectangle {
        anchors.fill: parent
        color: "#000000"
        opacity: root.open ? 0.5 : 0.0

        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

        MouseArea {
            anchors.fill: parent
            enabled: root.open
            onClicked: { root.cancelled(); root.hide() }
        }
    }

    // Dialog card
    Rectangle {
        id: dialog
        anchors.centerIn: parent
        width: 480
        height: form.implicitHeight + Theme.sp4 * 2

        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusLg

        Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

        layer.enabled: true
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#000000"
            shadowOpacity: 0.5
            shadowBlur: 1.0
            shadowVerticalOffset: 16
        }

        ColumnLayout {
            id: form
            anchors.fill: parent
            anchors.margins: Theme.sp4
            spacing: Theme.sp3

            // Title
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp0_5

                SectionLabel { text: qsTr("Connection") }

                Text {
                    text: qsTr("New SSH connection")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeH2
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }
            }

            Separator { Layout.fillWidth: true; Layout.topMargin: Theme.sp1 }

            // Form fields
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                Text {
                    text: qsTr("Name")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }
                PierTextField {
                    id: nameField
                    Layout.fillWidth: true
                    placeholder: qsTr("My production server")
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp3

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp2
                    Text {
                        text: qsTr("Host")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        font.weight: Theme.weightMedium
                        color: Theme.textSecondary
                    }
                    PierTextField {
                        id: hostField
                        Layout.fillWidth: true
                        placeholder: qsTr("example.com")
                    }
                }

                ColumnLayout {
                    Layout.preferredWidth: 96
                    spacing: Theme.sp2
                    Text {
                        text: qsTr("Port")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        font.weight: Theme.weightMedium
                        color: Theme.textSecondary
                    }
                    PierTextField {
                        id: portField
                        Layout.fillWidth: true
                        text: "22"
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2
                Text {
                    text: qsTr("Username")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }
                PierTextField {
                    id: userField
                    Layout.fillWidth: true
                    placeholder: qsTr("root")
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2
                Text {
                    text: qsTr("Authentication")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }
                PierComboBox {
                    id: authCombo
                    Layout.fillWidth: true
                    // M3b wires Password end-to-end; Private key
                    // and SSH agent exist in the Rust enum but
                    // aren't plumbed through the dialog yet (M3c).
                    // Keep them visible so users know what's
                    // coming, but mark them.
                    options: [qsTr("Password"),
                              qsTr("Private key (M3c)"),
                              qsTr("SSH agent (M3c)")]
                    currentIndex: 0
                }
            }

            // Password field — shown only when "Password" is the
            // selected auth method. Bullet echo via PierTextField's
            // new `password` property.
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2
                visible: authCombo.currentIndex === 0
                Text {
                    text: qsTr("Password")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }
                PierTextField {
                    id: passwordField
                    Layout.fillWidth: true
                    placeholder: qsTr("password")
                    password: true
                }
            }

            Item { Layout.preferredHeight: Theme.sp1 }

            // Action buttons
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                Item { Layout.fillWidth: true }

                GhostButton {
                    text: qsTr("Cancel")
                    onClicked: { root.cancelled(); root.hide() }
                }

                PrimaryButton {
                    text: qsTr("Connect")
                    // Require the minimum set for a Password
                    // connection. Private key / agent will get
                    // their own validation in M3c.
                    enabled: nameField.text.length > 0
                             && hostField.text.length > 0
                             && userField.text.length > 0
                             && (authCombo.currentIndex !== 0
                                 || passwordField.text.length > 0)
                    onClicked: {
                        const conn = {
                            name: nameField.text,
                            host: hostField.text,
                            port: parseInt(portField.text) || 22,
                            username: userField.text,
                            authKind: authCombo.currentIndex === 0 ? "password"
                                    : authCombo.currentIndex === 1 ? "private_key"
                                    : "agent",
                            password: passwordField.text
                        }
                        root.saved(conn)
                        root.hide()
                    }
                }
            }
        }
    }
}
