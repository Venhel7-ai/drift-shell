import QtQuick
import QtQuick.Controls
import "../shell-ui/qml"
ApplicationWindow {
    id: win
    width: previewWidth; height: previewHeight
    visible: true
    title: "Drift Shell — предпросмотр, тестовые данные"
    color: "black"
    Stars { anchors.fill: parent; count: previewBackend.snapshot.config.starCount }
    Repeater {
        model: previewBackend.snapshot.previewWindows
        Surface {
            required property var modelData
            x: modelData.x; y: modelData.y; width: modelData.w; height: modelData.h
            materialRole: "App"
            border.color: modelData.active ? Theme.focus : Theme.border
            Column {
                x: 14; y: 14; spacing: 24
                Text { text: modelData.title; color: Theme.secondary; font.family: Theme.font; font.pixelSize: 11 }
                Text { text: modelData.body; color: Theme.text; font.family: Theme.monoFont; font.pixelSize: 13; lineHeight: 1.8 }
            }
        }
    }
    Desktop {
        anchors.fill: parent
        backendState: previewBackend.snapshot
        searchResults: previewBackend.results
        clockText: "22:41"
        dateText: "26.09.2026"
        onCommand: (action,payload) => previewBackend.command(action, JSON.stringify(payload))
        onSearch: query => previewBackend.search(query)
    }
}
