import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Wayland
import Quickshell.Services.Pam
import "../shell-ui/qml"
ShellRoot {
    id: root
    property date now: new Date()
    property bool failed: false
    property string secret: ""
    property bool attempted: false
    function submit(value) {
        if (!value.length || pam.active && !pam.responseRequired) return
        failed=false
        if (pam.active && pam.responseRequired) pam.respond(value)
        else { secret=value; attempted=true; pam.start() }
    }
    Timer { interval: 1000; repeat: true; running: true; onTriggered: root.now=new Date() }
    Timer { id: clearFailure; interval: 1500; onTriggered: root.failed=false }
    PamContext {
        id: pam
        config: "drift-shell"
        user: Quickshell.env("USER")
        onPamMessage: {
            if (responseRequired && root.attempted) {
                respond(root.secret)
                root.secret=""
                root.attempted=false
            }
        }
        onCompleted: result => {
            root.secret=""; root.attempted=false
            if (result === PamResult.Success) { lock.locked=false; Qt.quit() }
            else { root.failed=true; clearFailure.restart() }
        }
        onError: { root.secret=""; root.attempted=false; root.failed=true; clearFailure.restart() }
    }
    WlSessionLock {
        id: lock
        locked: true
        WlSessionLockSurface {
            color: "black"
            Stars { anchors.fill: parent }
            Text { x: 32; y: 24; text: "△  Arch Linux"; color: Theme.text; font.family: Theme.font; font.pixelSize: 14 }
            Column {
                id: column
                anchors.horizontalCenter: parent.horizontalCenter
                y: parent.height * .29
                spacing: 14
                Text { anchors.horizontalCenter: parent.horizontalCenter; text: Qt.formatTime(root.now,"hh:mm"); color: Theme.text; font.family: Theme.font; font.weight: Font.Light; font.pixelSize: 88 }
                Text { anchors.horizontalCenter: parent.horizontalCenter; text: Qt.formatDate(root.now,"dddd, d MMMM"); color: Theme.secondary; font.family: Theme.font; font.pixelSize: 23 }
                Item { width: 1; height: 24 }
                Rectangle {
                    anchors.horizontalCenter: parent.horizontalCenter; width: 78; height: 78; radius: 39
                    color: "#0dffffff"; border.color: Theme.focus
                    Text { anchors.centerIn: parent; text: Quickshell.env("USER").slice(0,1).toUpperCase(); color: Theme.text; font.family: Theme.font; font.pixelSize: 28 }
                }
                Text { anchors.horizontalCenter: parent.horizontalCenter; text: Quickshell.env("USER"); color: Theme.text; font.family: Theme.font; font.pixelSize: 14 }
                ShellField {
                    id: password
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 300; height: 40; focus: true
                    color: Theme.text
                    echoMode: pam.responseVisible ? TextInput.Normal : TextInput.Password
                    opacity: text.length || pam.active || root.failed ? 1 : 0
                    background: Surface { border.color: root.failed ? Theme.error : Theme.focus }
                    onAccepted: { root.submit(text); text="" }
                    Keys.onEscapePressed: { text=""; root.secret=""; root.attempted=false; pam.abort() }
                    Connections { target: pam; function onCompleted() { password.text="" } function onError() { password.text="" } }
                }
            }
        }
    }
}
