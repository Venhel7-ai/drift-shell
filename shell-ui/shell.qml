import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import "qml"
ShellRoot {
    id: shell
    property var snapshot: ({config: {}, panels: {}, zones: [], slots: [], tasks: [], metrics: {}, notifications: [], settings: {}, outputs: {}, compositor: {windows: [], outputs: []}})
    property var results: []
    property string lastError: ""
    property int sequence: 0
    property int searchRequest: -1
    property date now: new Date()
    function send(action, payload) {
        if (!ipc.connected) { lastError = "Нет соединения с ядром оболочки"; return -1 }
        const id = ++sequence
        ipc.write(JSON.stringify({schemaVersion:1, requestId:id, action:action, payload:payload}) + "\n")
        ipc.flush()
        return id
    }
    Timer { interval: 1000; running: true; repeat: true; onTriggered: shell.now = new Date() }
    Timer { interval: 1500; running: !ipc.connected; repeat: true; onTriggered: ipc.connected = true }
    Timer { id: clearError; interval: 5000; onTriggered: shell.lastError = "" }
    Socket {
        id: ipc
        path: Quickshell.env("XDG_RUNTIME_DIR") + "/drift-shell/core.sock"
        connected: true
        parser: SplitParser {
            onRead: data => {
                try {
                    const message=JSON.parse(data)
                    if (message.type === "State") shell.snapshot=message.payload
                    else if (message.ok === false) { shell.lastError=message.errorMessage + ": " + message.errorCode; clearError.restart() }
                    else if (message.requestId === shell.searchRequest) shell.results=message.result
                } catch(e) { shell.lastError="Ошибка протокола ядра" }
            }
        }
    }
    Process { id: lockProcess; command: ["/usr/bin/drift-shell-lock"] }
    Variants {
        model: Quickshell.screens
        Scope {
            required property var modelData
            property var output: (shell.snapshot.compositor.outputs || []).find(o => o.name === modelData.name)
            property bool fullscreen: (shell.snapshot.compositor.fullscreen || []).some(w => w.output === modelData.name)
            PanelWindow {
                screen: modelData
                anchors { top: true; bottom: true; left: true; right: true }
                WlrLayershell.layer: WlrLayer.Background
                WlrLayershell.namespace: "drift-shell-background"
                exclusionMode: ExclusionMode.Ignore
                mask: Region {}
                Stars {
                    anchors.fill: parent
                    count: shell.snapshot.config.starCount || 0
                    cameraX: output ? output.camera[0] : 0
                    cameraY: output ? output.camera[1] : 0
                }
            }
            PanelWindow {
                screen: modelData
                anchors { top: true; left: true; right: true }
                implicitHeight: 44
                visible: !fullscreen
                exclusiveZone: 44
                color: "transparent"
                WlrLayershell.namespace: "drift-shell-reserve"
                mask: Region {}
            }
            PanelWindow {
                id: overlay
                visible: !fullscreen
                screen: modelData
                anchors { top: true; bottom: true; left: true; right: true }
                color: "transparent"
                WlrLayershell.layer: WlrLayer.Overlay
                WlrLayershell.namespace: "drift-shell-ui"
                WlrLayershell.keyboardFocus: desktop.keyboardNeeded ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.OnDemand
                exclusionMode: ExclusionMode.Ignore
                mask: Region {
                    Region { item: desktop.topRegion }
                    Region { x: desktop.rightRegion.x; y: desktop.rightRegion.y; width: desktop.rightRegion.visible ? desktop.rightRegion.width : 0; height: desktop.rightRegion.height }
                    Region { x: desktop.dockRegion.x; y: desktop.dockRegion.y; width: desktop.dockRegion.visible ? desktop.dockRegion.width : 0; height: desktop.dockRegion.height }
                    Region { x: desktop.searchRegion.x; y: desktop.searchRegion.y; width: desktop.searchRegion.visible ? desktop.searchRegion.width : 0; height: desktop.searchRegion.height }
                    Region { x: desktop.settingsRegion.x; y: desktop.settingsRegion.y; width: desktop.settingsRegion.visible ? desktop.settingsRegion.width : 0; height: desktop.settingsRegion.height }
                    Region { x: desktop.notificationRegion.x; y: desktop.notificationRegion.y; width: desktop.notificationRegion.visible ? desktop.notificationRegion.width : 0; height: desktop.notificationRegion.height }
                    Region { x: desktop.toastRegion.x; y: desktop.toastRegion.y; width: desktop.toastRegion.visible ? desktop.toastRegion.width : 0; height: desktop.toastRegion.height }
                    Region { x: desktop.calendarRegion.x; y: desktop.calendarRegion.y; width: desktop.calendarRegion.visible ? desktop.calendarRegion.width : 0; height: desktop.calendarRegion.height }
                }
                Desktop {
                    id: desktop
                    anchors.fill: parent
                    backendState: shell.snapshot
                    activeOutput: output ? output.active : true
                    searchResults: shell.results
                    errorText: shell.lastError
                    clockText: Qt.formatTime(shell.now, "hh:mm")
                    dateText: Qt.formatDate(shell.now, "dd.MM.yyyy")
                    onCommand: (action,payload) => shell.send(action,payload)
                    onSearch: query => { shell.searchRequest = shell.send("search", {query:query}) }
                    onLockRequested: if (!lockProcess.running) lockProcess.running=true
                }
            }
        }
    }
}
