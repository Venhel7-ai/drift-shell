import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
Surface {
    id: view
    property var backendState: ({config: {}, settings: {}})
    signal command(string action, var payload)
    property string selectedKey: ""
    materialRole: "Overlay"
    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 22
        spacing: 14
        RowLayout {
            Layout.fillWidth: true
            Text { text: "Настройки системы"; color: Theme.text; font.family: Theme.font; font.pixelSize: 22; Layout.fillWidth: true }
            ShellButton { text: "×"; onClicked: view.command("dismiss", {}) }
        }
        RowLayout {
            Layout.fillWidth: true
            ShellField {
                id: filter
                Layout.fillWidth: true
                placeholderText: "Найти параметр на любом уровне…"
                color: Theme.text
                font.family: Theme.font
                background: Surface { materialRole: "App" }
            }
            ComboBox {
                id: level
                model: ["Основные", "Расширенные", "Экспертные"]
                palette.buttonText: Theme.text
                palette.text: Theme.text
                palette.button: Theme.app
                palette.base: Theme.app
                palette.highlight: Theme.hover
            }
        }
        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            ColumnLayout {
                width: view.width - 52
                spacing: 8
                Repeater {
                    model: Object.keys(view.backendState.settings || {})
                    delegate: Surface {
                        id: row
                        required property string modelData
                        property var meta: view.backendState.settings[modelData]
                        property var value: view.backendState.config[modelData]
                        visible: filter.text.length > 0 ? (meta.label + " " + modelData).toLowerCase().includes(filter.text.toLowerCase()) : meta.level <= level.currentIndex
                        Layout.fillWidth: true
                        implicitHeight: visible ? 66 : 0
                        border.color: view.selectedKey === modelData ? Theme.focus : Theme.border
                        RowLayout {
                            anchors.fill: parent; anchors.margins: 12
                            ColumnLayout {
                                Layout.fillWidth: true
                                Text { text: row.meta.label; color: Theme.text; font.pixelSize: Theme.bodySize; font.family: Theme.font }
                                Text { text: row.meta.group; color: Theme.tertiary; font.pixelSize: 10; font.family: Theme.font }
                            }
                            Switch {
                                visible: row.meta.type === "bool"
                                checked: row.value === true
                                onClicked: { const changes = {}; changes[row.modelData] = checked; view.command("configure", {changes: changes}) }
                                indicator: Rectangle {
                                    implicitWidth: 34; implicitHeight: 18; radius: 9
                                    color: parent.checked ? "#80ffffff" : "#26ffffff"
                                    Rectangle { x: parent.parent.checked ? 18 : 2; y: 2; width: 14; height: 14; radius: 7; color: "white" }
                                }
                            }
                            ComboBox {
                                visible: row.meta.type === "enum"
                                model: row.meta.choiceLabels || row.meta.choices || []
                                currentIndex: (row.meta.choices || []).indexOf(row.value)
                                palette.button: Theme.app; palette.buttonText: Theme.text; palette.text: Theme.text; palette.base: Theme.app; palette.highlight: Theme.hover
                                onActivated: { const changes = {}; changes[row.modelData] = row.meta.choices[currentIndex]; view.command("configure", {changes: changes}) }
                            }
                            ShellField {
                                visible: row.meta.type === "int" || row.meta.type === "number"
                                text: String(row.value)
                                color: Theme.text
                                Layout.preferredWidth: 90
                                background: Surface { materialRole: "App" }
                                onEditingFinished: {
                                    const n = Number(text)
                                    if (isFinite(n)) { const changes = {}; changes[row.modelData] = n; view.command("configure", {changes: changes}) }
                                }
                            }
                            ShellButton {
                                text: "Сброс"
                                onClicked: { const changes = {}; changes[row.modelData] = row.meta.default; view.command("configure", {changes: changes}) }
                            }
                        }
                    }
                }
                Text {
                    Layout.fillWidth: true
                    text: "Соединение с композитором: " + (view.backendState.connected ? "установлено" : "нет") + "\nПринятая раскладка: " + (view.backendState.acceptedRevision || 0)
                    color: Theme.secondary; font.family: Theme.monoFont; font.pixelSize: 11
                }
                Repeater {
                    model: view.backendState.errors || []
                    delegate: Text { required property string modelData; text: modelData; color: Theme.error; font.pixelSize: 11; wrapMode: Text.Wrap; Layout.fillWidth: true }
                }
            }
        }
    }
}
