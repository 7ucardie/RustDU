// Renders a simple app icon (drive glyph on a gradient tile) to a PNG.
// Usage: swift Tools/make-icon.swift /path/to/out.png
import AppKit

let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "icon.png"
let size: CGFloat = 1024
let image = NSImage(size: NSSize(width: size, height: size))
image.lockFocus()
let inset: CGFloat = size * 0.09
let tile = NSBezierPath(roundedRect: NSRect(x: inset, y: inset, width: size - 2 * inset, height: size - 2 * inset),
                        xRadius: size * 0.2, yRadius: size * 0.2)
NSGradient(colors: [NSColor(calibratedRed: 0.17, green: 0.42, blue: 0.95, alpha: 1),
                    NSColor(calibratedRed: 0.08, green: 0.16, blue: 0.42, alpha: 1)])!
    .draw(in: tile, angle: -70)

// Three stacked "usage bars" of decreasing length.
let barX = size * 0.24, barW = size * 0.52, barH = size * 0.085, gap = size * 0.06
let fractions: [CGFloat] = [1.0, 0.62, 0.3]
let colors = [NSColor.white, NSColor.white.withAlphaComponent(0.78), NSColor.white.withAlphaComponent(0.55)]
for (i, f) in fractions.enumerated() {
    let y = size * 0.62 - CGFloat(i) * (barH + gap)
    NSColor.white.withAlphaComponent(0.18).setFill()
    NSBezierPath(roundedRect: NSRect(x: barX, y: y, width: barW, height: barH), xRadius: barH / 2, yRadius: barH / 2).fill()
    colors[i].setFill()
    NSBezierPath(roundedRect: NSRect(x: barX, y: y, width: barW * f, height: barH), xRadius: barH / 2, yRadius: barH / 2).fill()
}
image.unlockFocus()

guard let tiff = image.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff),
      let png = rep.representation(using: .png, properties: [:]) else {
    fputs("failed to render icon\n", stderr); exit(1)
}
try! png.write(to: URL(fileURLWithPath: out))
print("wrote \(out)")
