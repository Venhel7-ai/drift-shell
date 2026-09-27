pragma Singleton
import QtQuick
QtObject {
    readonly property color background: "#000000"
    readonly property color app: "#f0050607"
    readonly property color shell: "#d1080a0b"
    readonly property color overlay: "#e0080a0b"
    readonly property color text: "#ebffffff"
    readonly property color secondary: "#a8ffffff"
    readonly property color tertiary: "#75ffffff"
    readonly property color border: "#1cffffff"
    readonly property color focus: "#3dffffff"
    readonly property color success: "#5CFF8B"
    readonly property color error: "#FF5C64"
    readonly property color hover: "#0effffff"
    readonly property int radius: 12
    readonly property int topHeight: 38
    readonly property int topMargin: 6
    readonly property int searchWidth: 248
    readonly property int searchOpenWidth: 620
    readonly property int rightWidth: 290
    readonly property int dockHeight: 58
    readonly property int dockItem: 42
    readonly property int dockIcon: 23
    readonly property int slot: 32
    readonly property string font: "Inter"
    readonly property string monoFont: "JetBrains Mono"
    readonly property int bodySize: 12
    readonly property int titleSize: 15
    readonly property int motionPanelShow: 205
    readonly property int motionPanelHide: 185
    readonly property int motionDockShow: 195
    readonly property int motionDockHide: 175
    readonly property int motionSearch: 160
    readonly property int motionHover: 90
    readonly property int motionPress: 65
    readonly property int motionCamera: 250
    readonly property int starSeed: 858103
}
