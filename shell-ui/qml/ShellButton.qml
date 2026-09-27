import QtQuick
import QtQuick.Controls
Button {
    id: control
    property bool selected: false
    property color textColor: Theme.text
    implicitHeight: 34
    implicitWidth: Math.max(34, contentItem.implicitWidth + 20)
    font.family: Theme.font
    font.pixelSize: Theme.bodySize
    hoverEnabled: true
    background: Rectangle {
        radius: 8
        color: control.down ? "#18ffffff" : control.hovered || control.selected ? Theme.hover : "transparent"
        border.width: 1
        border.color: control.activeFocus || control.selected ? Theme.focus : "transparent"
    }
    contentItem: Text {
        text: control.text
        font: control.font
        color: control.textColor
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
    opacity: enabled ? 1 : .38
    Accessible.name: text
}
