import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Wayland
import Quickshell.Services.Greetd
import "../shell-ui/qml"
ShellRoot {
    id: root
    property string pending: ""
    property bool failed: false
    property bool waiting: false
    Connections {
        target: Greetd
        function onAuthMessage(message, error, responseRequired, echoResponse) {
            if (responseRequired) {
                if (root.pending.length) { Greetd.respond(root.pending); root.pending="" }
                else root.waiting=true
            }
        }
        function onAuthFailure(message) { root.pending=""; root.waiting=false; root.failed=true }
        function onError(message) { root.pending=""; root.waiting=false; root.failed=true }
        function onReadyToLaunch() { root.pending=""; Greetd.launch(["/usr/bin/drift-shell-session"]) }
    }
    Variants {
        model: Quickshell.screens
        PanelWindow {
            required property var modelData
            screen: modelData
            anchors { top: true; bottom: true; left: true; right: true }
            WlrLayershell.layer: WlrLayer.Overlay
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive
            color: "black"
            Stars { anchors.fill: parent }
            Text { x: 32; y: 24; text: "△  Arch Linux"; color: Theme.text; font.pixelSize: 14; font.family: Theme.font }
            Column {
                anchors.centerIn: parent; spacing: 16
                Rectangle { anchors.horizontalCenter: parent.horizontalCenter; width: 78; height: 78; radius: 39; color: "#0dffffff"; border.color: Theme.border }
                ShellField { id: user; width: 300; color: Theme.text; placeholderText: "Пользователь"; background: Surface {} }
                ShellField {
                    id: password; width: 300; height: 40; echoMode: TextInput.Password; focus: true; color: Theme.text
                    background: Surface { border.color: root.failed ? Theme.error : Theme.focus }
                    onAccepted: {
                        if (!text.length || !user.text.length) return
                        root.failed=false
                        if (root.waiting) { Greetd.respond(text); root.waiting=false }
                        else { root.pending=text; Greetd.createSession(user.text) }
                        text=""
                    }
                }
            }
        }
    }
}
