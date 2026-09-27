import QtQuick
Rectangle {
    property string materialRole: "Shell"
    color: materialRole === "Overlay" ? Theme.overlay : materialRole === "App" ? Theme.app : Theme.shell
    radius: Theme.radius
    border.width: 1
    border.color: Theme.border
}
