import QtQuick
Canvas {
    id: stars
    property int count: 38
    property real cameraX: 0
    property real cameraY: 0
    onCountChanged: requestPaint()
    onCameraXChanged: requestPaint()
    onCameraYChanged: requestPaint()
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
    onPaint: {
        const ctx = getContext("2d")
        ctx.fillStyle = Theme.background
        ctx.fillRect(0, 0, width, height)
        let seed = Theme.starSeed
        function random() { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 4294967296 }
        for (let i = 0; i < count; i++) {
            const x = ((random() * width - cameraX * .08) % width + width) % width
            const y = ((random() * height + cameraY * .08) % height + height) % height
            const p = random()
            const radius = p < .75 ? .55 : p < .93 ? .9 : p < .99 ? 1.25 : 1.8
            ctx.globalAlpha = .22 + random() * .6
            ctx.fillStyle = "white"
            ctx.beginPath(); ctx.arc(x, y, radius, 0, 2 * Math.PI); ctx.fill()
        }
        ctx.globalAlpha = 1
    }
}
