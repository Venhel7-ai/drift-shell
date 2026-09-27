import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
Item {
    id: root
    property var backendState: ({config: {}, panels: {}, zones: [], slots: [], tasks: [], metrics: {}, notifications: [], settings: {}, compositor: {windows: []}})
    property var searchResults: []
    property string errorText: ""
    property string clockText: Qt.formatTime(new Date(), "hh:mm")
    property string dateText: Qt.formatDate(new Date(), "dd.MM.yyyy")
    property bool activeOutput: true
    property bool zonesExpanded: false
    property bool metricsExpanded: false
    property bool showCalendar: false
    property bool addingServer: false
    property bool reference: ["reference", "showcase"].includes(backendState.config.profile)
    property bool reduced: backendState.config.reducedMotion === true
    property alias topRegion: top
    property alias rightRegion: right
    property alias dockRegion: dock
    property alias searchRegion: palette
    property alias settingsRegion: settings
    property alias notificationRegion: center
    property alias toastRegion: toasts
    property alias calendarRegion: calendar
    property bool keyboardNeeded: activeOutput && (backendState.panels.search || backendState.panels.settings || backendState.panels.notifications || backendState.panels.right === "Peek")
    signal command(string action, var payload)
    signal search(string query)
    signal lockRequested()
    function activate(result) {
        if (!result) return
        if (result.kind === "setting") {
            settings.selectedKey = result.id
            root.command("settings", {})
        } else root.command(result.kind === "app" ? "launch" : result.kind, {id: result.id})
    }
    Keys.onEscapePressed: { root.command("dismiss", {}); root.zonesExpanded = false; root.showCalendar = false }
    onKeyboardNeededChanged: if (keyboardNeeded) root.forceActiveFocus()
    Item {
        id: top
        width: parent.width; height: 44
        Row {
            anchors.left: parent.left; anchors.leftMargin: 16; y: Theme.topMargin; spacing: 10
            ShellButton { text: "△  Arch Linux"; height: Theme.topHeight; onClicked: root.command("settings", {}) }
            Surface {
                height: Theme.topHeight
                width: slotRow.width + 8
                Row {
                    id: slotRow; anchors.centerIn: parent
                    Repeater {
                        model: root.reference || root.zonesExpanded ? [1,2,3,4,5,6,7,8,9] : root.backendState.slots || []
                        ShellButton {
                            required property int modelData
                            width: Theme.slot; height: 30; text: String(modelData)
                            selected: (root.backendState.zones || []).some(z => z.id === root.backendState.activeZone && z.slot === modelData)
                            onClicked: root.command("zone", {slot: modelData})
                        }
                    }
                    ShellButton { visible: !root.reference; width: 32; height: 30; text: "···"; onClicked: root.zonesExpanded = !root.zonesExpanded }
                }
            }
        }
        Surface {
            id: searchTrigger
            y: Theme.topMargin; anchors.horizontalCenter: parent.horizontalCenter
            width: Math.min(root.reference ? 474 : Theme.searchWidth, root.width * .29)
            height: Theme.topHeight
            ShellButton { anchors.fill: parent; text: "⌕   Поиск…"; onClicked: root.command("search-toggle", {}) }
        }
        Surface {
            anchors.right: parent.right; anchors.rightMargin: 16; y: Theme.topMargin
            height: Theme.topHeight; width: statusRow.width + 12
            Row {
                id: statusRow; anchors.centerIn: parent
                ShellButton { text: root.backendState.panels.dnd ? "○" : "◉"; onClicked: root.command("notifications", {}); Accessible.name: "Уведомления" }
                ShellButton { text: root.backendState.compositor.layout_short || ""; enabled: false }
                ShellButton { text: (root.backendState.config.showDate ? root.dateText + "  " : "") + root.clockText; onClicked: root.showCalendar = !root.showCalendar }
                ShellButton { text: "☷"; onClicked: root.command("right", {}); Accessible.name: "Правая панель" }
            }
        }
    }
    Surface {
        id: calendar
        visible: root.showCalendar
        x: parent.width - width - 16; y: 54; width: 250; height: 100
        Text { anchors.centerIn: parent; text: root.dateText; color: Theme.text; font.family: Theme.font; font.pixelSize: 23 }
    }
    Item {
        id: right
        width: root.backendState.config.rightWidth || Theme.rightWidth
        height: Math.max(0, root.height - 76)
        y: 56
        x: root.backendState.panels.right === "Hidden" || !root.backendState.panels.right ? root.width + 4 : root.width - width - 12
        visible: x < root.width
        Behavior on x { NumberAnimation { duration: root.reduced ? 60 : root.backendState.panels.right === "Hidden" ? Theme.motionPanelHide : Theme.motionPanelShow; easing.type: Easing.BezierSpline; easing.bezierCurve: [.2,.8,.2,1,1,1] } }
        ScrollView {
            anchors.fill: parent; clip: true
            ColumnLayout {
                width: right.width; spacing: 12
                Surface {
                    Layout.fillWidth: true
                    implicitHeight: systemColumn.implicitHeight + 28
                    ColumnLayout {
                        id: systemColumn; anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top; anchors.margins: 14; spacing: 9
                        RowLayout {
                            Layout.fillWidth: true
                            Text { text: "Система"; color: Theme.text; font.pixelSize: Theme.titleSize; font.family: Theme.font; Layout.fillWidth: true }
                            ShellButton { text: root.backendState.panels.right === "Pinned" ? "−" : "+"; onClicked: root.command("pin", {}); Accessible.name: "Закрепить панель" }
                            ShellButton { text: "×"; onClicked: root.command("right", {}) }
                        }
                        Repeater {
                            model: [{key:"cpu",name:"CPU"},{key:"memory",name:"Память"},{key:"disk",name:"Диск"}]
                            delegate: ColumnLayout {
                                required property var modelData
                                Layout.fillWidth: true; spacing: 5
                                RowLayout {
                                    Layout.fillWidth: true
                                    Text { text: modelData.name; color: Theme.secondary; font.family: Theme.font; font.pixelSize: 12; Layout.fillWidth: true }
                                    Text { text: root.backendState.metrics[modelData.key] === undefined ? "—" : root.backendState.metrics[modelData.key] + "%"; color: Theme.text; font.family: Theme.monoFont; font.pixelSize: 11 }
                                }
                                Rectangle {
                                    Layout.fillWidth: true; height: 3; radius: 2; color: "#18ffffff"
                                    Rectangle { width: parent.width * (root.backendState.metrics[modelData.key] || 0) / 100; height: parent.height; radius: 2; color: Theme.secondary }
                                }
                            }
                        }
                        Text {
                            text: "Сеть   ↓ " + ((root.backendState.metrics.download || 0)/1024).toFixed(0) + " КБ/с   ↑ " + ((root.backendState.metrics.upload || 0)/1024).toFixed(0) + " КБ/с"
                            color: Theme.secondary; font.family: Theme.font; font.pixelSize: 11
                        }
                        ShellButton { text: root.metricsExpanded ? "Свернуть" : "Подробно"; onClicked: root.metricsExpanded = !root.metricsExpanded }
                        Canvas {
                            id: graph
                            Layout.fillWidth: true; height: 70
                            property var samples: root.backendState.samples || []
                            visible: root.backendState.config.graphPolicy !== "Off" && (root.metricsExpanded || root.backendState.config.graphPolicy === "Always" || (root.backendState.config.graphPolicy === "OnHover" && graphHover.hovered))
                            onSamplesChanged: if (visible) requestPaint()
                            onVisibleChanged: if (visible) requestPaint()
                            onPaint: {
                                const c=getContext("2d"); c.clearRect(0,0,width,height); c.strokeStyle=Theme.secondary; c.lineWidth=1.3; c.beginPath()
                                for (let i=0; i<samples.length; i++) { const x=i*width/Math.max(1,samples.length-1), y=height-samples[i]*height/100; if(i===0)c.moveTo(x,y);else c.lineTo(x,y) }
                                c.stroke()
                            }
                        }
                    }
                    HoverHandler { id: graphHover }
                }
                Surface {
                    Layout.fillWidth: true; implicitHeight: serverColumn.implicitHeight + 28
                    ColumnLayout {
                        id: serverColumn; anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right; anchors.margins: 14; spacing: 8
                        RowLayout {
                            Layout.fillWidth: true
                            Text { text: "Серверы"; color: Theme.text; font.family: Theme.font; font.pixelSize: Theme.titleSize; Layout.fillWidth: true }
                            ShellButton { text: "+"; onClicked: root.addingServer = !root.addingServer }
                        }
                        Text { visible: !(root.backendState.servers || []).length; text: "Серверы не добавлены"; color: Theme.tertiary; font.pixelSize: 12 }
                        Repeater {
                            model: root.backendState.servers || []
                            delegate: RowLayout {
                                required property var modelData
                                Layout.fillWidth: true
                                Rectangle { width: 8; height: 8; radius: 4; color: modelData.status === "online" ? Theme.success : modelData.status === "offline" ? Theme.error : Theme.tertiary }
                                Text { text: modelData.name; color: Theme.text; font.pixelSize: 12; Layout.fillWidth: true; elide: Text.ElideRight }
                                Text { text: modelData.latency === undefined || modelData.latency === null ? "—" : modelData.latency + " мс"; color: Theme.secondary; font.pixelSize: 11 }
                                ShellButton { text: "×"; onClicked: root.command("server-delete", {id: modelData.id}) }
                            }
                        }
                        ShellField { id: serverName; visible: root.addingServer; Layout.fillWidth: true; placeholderText: "Название"; color: Theme.text; background: Surface { materialRole: "App" } }
                        RowLayout {
                            visible: root.addingServer
                            Layout.fillWidth: true
                            ShellField { id: serverHost; Layout.fillWidth: true; placeholderText: "Адрес"; color: Theme.text; background: Surface { materialRole: "App" } }
                            ShellField { id: serverPort; Layout.preferredWidth: 64; text: "443"; color: Theme.text; background: Surface { materialRole: "App" } }
                        }
                        ShellButton {
                            visible: root.addingServer
                            text: "Добавить TCP-проверку"
                            onClicked: {
                                root.command("server-add", {name:serverName.text.trim(),host:serverHost.text.trim(),port:Number(serverPort.text)})
                                serverName.text=""; serverHost.text=""
                            }
                        }
                    }
                }
                Surface {
                    Layout.fillWidth: true; implicitHeight: tasksColumn.implicitHeight + 28
                    ColumnLayout {
                        id: tasksColumn; anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right; anchors.margins: 14; spacing: 8
                        Text { text: "Задачи"; color: Theme.text; font.family: Theme.font; font.pixelSize: Theme.titleSize }
                        Text { visible: !(root.backendState.tasks || []).length; text: "Список пуст"; color: Theme.tertiary; font.family: Theme.font; font.pixelSize: 12 }
                        Repeater {
                            model: root.backendState.tasks || []
                            delegate: RowLayout {
                                required property var modelData
                                Layout.fillWidth: true
                                ShellButton { text: modelData.done ? "✓" : "□"; textColor: modelData.done ? Theme.success : Theme.secondary; onClicked: root.command("task-toggle", {id: modelData.id}) }
                                Text { text: modelData.title; color: Theme.text; font.family: Theme.font; font.pixelSize: 12; font.strikeout: modelData.done; wrapMode: Text.Wrap; Layout.fillWidth: true }
                                ShellButton { text: "×"; onClicked: root.command("task-delete", {id: modelData.id}) }
                            }
                        }
                        ShellField {
                            Layout.fillWidth: true; placeholderText: "Новая задача"; color: Theme.text
                            background: Surface { materialRole: "App" }
                            onAccepted: { if(text.trim()) root.command("task-add", {title: text}); text = "" }
                        }
                    }
                }
                Surface {
                    Layout.fillWidth: true; implicitHeight: 60
                    Row { anchors.centerIn: parent
                        ShellButton { text: "Настройки"; onClicked: root.command("settings", {}) }
                        ShellButton { text: "Фокус"; selected: root.backendState.focus === true; onClicked: root.command("focus", {}) }
                    }
                }
            }
        }
    }
    Surface {
        id: dock
        height: Theme.dockHeight; width: Math.min(root.width - 32, dockRow.width + 32)
        anchors.horizontalCenter: parent.horizontalCenter
        y: root.backendState.panels.dock === "Hidden" || !root.backendState.panels.dock ? root.height + 4 : root.height - height - 12
        visible: y < root.height
        Behavior on y { NumberAnimation { duration: root.reduced ? 60 : root.backendState.panels.dock === "Hidden" ? Theme.motionDockHide : Theme.motionDockShow; easing.type: Easing.BezierSpline; easing.bezierCurve: [.2,.8,.2,1,1,1] } }
        Row {
            id: dockRow; anchors.centerIn: parent; spacing: 10
            ShellButton { text: "⌕"; width: 42; height: 42; onClicked: root.command("search-toggle", {}); Accessible.name: "Приложения" }
            Repeater {
                model: (root.backendState.compositor.windows || []).slice(0, 7)
                ShellButton { required property var modelData; text: (modelData.app_id || "?").slice(0, 2).toUpperCase(); selected: modelData.is_focused; width: 42; height: 42; onClicked: root.command("window", {id: modelData.id}); Accessible.name: modelData.title || modelData.app_id }
            }
            ShellButton { text: "⚙"; width: 42; height: 42; onClicked: root.command("settings", {}); Accessible.name: "Настройки" }
            ShellButton { text: "◇"; width: 42; height: 42; onClicked: root.lockRequested(); Accessible.name: "Заблокировать" }
        }
    }
    Surface {
        id: palette
        materialRole: "Overlay"
        visible: root.activeOutput && root.backendState.panels.search === true
        width: Math.min(Theme.searchOpenWidth, root.width - 32)
        height: Math.min(500, root.height - 100)
        anchors.horizontalCenter: parent.horizontalCenter; y: 56
        onVisibleChanged: if (visible) { query.forceActiveFocus(); query.text = ""; root.search("") }
        ColumnLayout {
            anchors.fill: parent; anchors.margins: 12; spacing: 8
            ShellField {
                id: query; Layout.fillWidth: true; color: Theme.text; placeholderText: "Приложения, окна, области, настройки…"
                font.family: Theme.font; font.pixelSize: 13
                background: Surface { materialRole: "App" }
                onTextChanged: debounce.restart()
                onAccepted: root.activate(root.searchResults[results.currentIndex])
                Keys.onDownPressed: results.currentIndex = Math.min(results.count-1, results.currentIndex+1)
                Keys.onUpPressed: results.currentIndex = Math.max(0, results.currentIndex-1)
                Timer { id: debounce; interval: 40; onTriggered: root.search(query.text) }
            }
            ListView {
                id: results; Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                model: root.searchResults
                spacing: 3; currentIndex: 0
                delegate: ShellButton {
                    required property var modelData; required property int index
                    width: results.width; height: 44; text: modelData.title
                    selected: results.currentIndex === index
                    onClicked: root.activate(modelData)
                }
            }
        }
    }
    SettingsView {
        id: settings
        visible: root.activeOutput && root.backendState.panels.settings === true
        backendState: root.backendState
        width: Math.min(1000, root.width - 48); height: Math.min(780, root.height - 100)
        anchors.centerIn: parent
        onCommand: (action,payload) => root.command(action,payload)
    }
    Surface {
        id: center; materialRole: "Overlay"
        visible: root.activeOutput && root.backendState.panels.notifications === true
        width: Math.min(378, root.width - 32); height: root.height - 80
        x: root.width-width-16; y: 56
        ColumnLayout {
            anchors.fill: parent; anchors.margins: 14
            Text { text: "Уведомления"; color: Theme.text; font.family: Theme.font; font.pixelSize: Theme.titleSize }
            RowLayout {
                ShellButton { text: "Очистить всё"; onClicked: root.command("notification-clear", {}) }
                ShellButton { text: "Не беспокоить"; selected: root.backendState.panels.dnd === true; onClicked: root.command("dnd", {}) }
            }
            Text { visible: root.backendState.notificationAvailable === false; text: "Служба уведомлений недоступна"; color: Theme.secondary; font.pixelSize: 11 }
            ListView {
                Layout.fillWidth: true; Layout.fillHeight: true; clip: true; spacing: 10
                model: (root.backendState.notifications || []).slice().reverse()
                delegate: notificationCard
            }
        }
    }
    Column {
        id: toasts; width: 330; spacing: 10; x: root.width-width-16; y: 56
        visible: root.activeOutput && !center.visible && !settings.visible && !palette.visible
        Repeater { model: (root.backendState.popups || []).slice(-3).reverse(); delegate: notificationCard }
    }
    Component {
        id: notificationCard
        Surface {
            required property var modelData
            width: 330; implicitHeight: notificationColumn.implicitHeight + 24
            ColumnLayout {
                id: notificationColumn; anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top; anchors.margins: 12; spacing: 6
                Text { text: modelData.app; color: Theme.tertiary; font.pixelSize: 10; textFormat: Text.PlainText }
                Text { text: modelData.title; color: Theme.text; font.pixelSize: 13; textFormat: Text.PlainText; wrapMode: Text.Wrap; Layout.fillWidth: true }
                Text { text: modelData.body; color: Theme.secondary; font.pixelSize: 12; textFormat: Text.PlainText; maximumLineCount: 3; elide: Text.ElideRight; wrapMode: Text.Wrap; Layout.fillWidth: true }
                Flow {
                    Layout.fillWidth: true; spacing: 4
                    Repeater {
                        model: modelData.actions
                        ShellButton { required property var modelData; text: modelData.title; onClicked: root.command("notification-action", {id: notificationColumn.parent.modelData.id, action: modelData.id}) }
                    }
                }
            }
        }
    }
    Text {
        anchors.horizontalCenter: parent.horizontalCenter; y: 48; text: root.errorText
        color: Theme.error; visible: text.length > 0; font.family: Theme.font; font.pixelSize: 12
    }
}
