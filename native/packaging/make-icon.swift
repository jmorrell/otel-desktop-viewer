import AppKit

let output = CommandLine.arguments[1]
let side = 1024
let bitmap = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: side,
    pixelsHigh: side,
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
NSGraphicsContext.current?.imageInterpolation = .high

let bounds = NSRect(x: 0, y: 0, width: side, height: side)
let card = NSBezierPath(roundedRect: bounds.insetBy(dx: 18, dy: 18), xRadius: 210, yRadius: 210)
NSColor(calibratedRed: 0.055, green: 0.09, blue: 0.17, alpha: 1).setFill()
card.fill()

let center = NSPoint(x: 512, y: 512)
for (diameter, alpha, width) in [(610.0, 0.22, 32.0), (430.0, 0.38, 34.0), (250.0, 0.72, 38.0)] {
    let ring = NSBezierPath(ovalIn: NSRect(x: center.x - diameter / 2, y: center.y - diameter / 2, width: diameter, height: diameter))
    ring.lineWidth = width
    NSColor(calibratedRed: 0.30, green: 0.80, blue: 0.98, alpha: alpha).setStroke()
    ring.stroke()
}

for (x, y, diameter) in [(512.0, 512.0, 118.0), (512.0, 817.0, 70.0), (776.0, 374.0, 70.0), (248.0, 374.0, 70.0)] {
    let dot = NSBezierPath(ovalIn: NSRect(x: x - diameter / 2, y: y - diameter / 2, width: diameter, height: diameter))
    NSColor(calibratedRed: 0.52, green: 0.91, blue: 0.98, alpha: 1).setFill()
    dot.fill()
}

NSGraphicsContext.restoreGraphicsState()
let data = bitmap.representation(using: .png, properties: [:])!
try data.write(to: URL(fileURLWithPath: output))
