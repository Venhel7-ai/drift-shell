import QtQuick
import QtQuick.Controls
TextField {
    color: Theme.text
    placeholderTextColor: Theme.tertiary
    selectionColor: "#40ffffff"
    selectedTextColor: Theme.text
    font.family: Theme.font
    font.pixelSize: Theme.bodySize
    implicitHeight: 34
    leftPadding: 10
    rightPadding: 10
    background: Surface { materialRole: "App"; border.color: parent.activeFocus ? Theme.focus : Theme.border }
}
