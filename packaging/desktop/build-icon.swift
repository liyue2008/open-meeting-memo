// Original OMM artwork, Apache-2.0. Geometry matches docs/assets/omm-logo.svg.
// macOS application icons are static; use the approved light brand colors.
import AppKit

let destination = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: true)
for (name, size) in [("icon_16x16", 16), ("icon_16x16@2x", 32), ("icon_32x32", 32),
                     ("icon_32x32@2x", 64), ("icon_128x128", 128), ("icon_128x128@2x", 256),
                     ("icon_256x256", 256), ("icon_256x256@2x", 512), ("icon_512x512", 512),
                     ("icon_512x512@2x", 1024)] {
    let context = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8,
                            bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    context.translateBy(x: Double(size) * 0.1, y: Double(size) * 0.9)
    context.scaleBy(x: Double(size) * 0.8 / 43, y: -Double(size) * 0.8 / 43)
    context.setFillColor(CGColor(red: 0, green: 133.0 / 255, blue: 115.0 / 255, alpha: 1))
    context.addPath(CGPath(roundedRect: CGRect(x: 0, y: 0, width: 43, height: 43),
                           cornerWidth: 12, cornerHeight: 12, transform: nil))
    context.fillPath()
    context.translateBy(x: 3.44, y: 12.0774)
    context.scaleBy(x: 36.12 / 46, y: 18.8452 / 24)
    context.setLineWidth(2.5)
    context.setLineCap(.round)
    context.setLineJoin(.round)
    let o = CGMutablePath()
    o.move(to: CGPoint(x: 7, y: 6.5))
    o.addLine(to: CGPoint(x: 8.5, y: 6.5))
    o.addLine(to: CGPoint(x: 11, y: 9))
    o.addLine(to: CGPoint(x: 11, y: 13.5))
    o.addCurve(to: CGPoint(x: 7, y: 17.5), control1: CGPoint(x: 11, y: 15.7), control2: CGPoint(x: 9.2, y: 17.5))
    o.addCurve(to: CGPoint(x: 3, y: 13.5), control1: CGPoint(x: 4.8, y: 17.5), control2: CGPoint(x: 3, y: 15.7))
    o.addLine(to: CGPoint(x: 3, y: 10.5))
    o.addCurve(to: CGPoint(x: 7, y: 6.5), control1: CGPoint(x: 3, y: 8.3), control2: CGPoint(x: 4.8, y: 6.5))
    o.closeSubpath()
    context.setStrokeColor(CGColor(red: 0, green: 3.0 / 255, blue: 4.0 / 255, alpha: 1))
    context.addPath(o)
    context.strokePath()
    context.setStrokeColor(CGColor(gray: 1, alpha: 1))
    for x in [15.0, 31.0] {
        let m = CGMutablePath()
        m.move(to: CGPoint(x: x, y: 17.5))
        m.addLine(to: CGPoint(x: x, y: 7.5))
        for offset in [0.0, 6.0] {
            m.move(to: CGPoint(x: x + offset, y: 10.5))
            m.addCurve(to: CGPoint(x: x + offset + 6, y: 10.5),
                       control1: CGPoint(x: x + offset, y: 5.7), control2: CGPoint(x: x + offset + 6, y: 5.7))
            m.addLine(to: CGPoint(x: x + offset + 6, y: 17.5))
        }
        context.addPath(m)
        context.strokePath()
    }
    let bitmap = NSBitmapImageRep(cgImage: context.makeImage()!)
    try bitmap.representation(using: .png, properties: [:])!.write(to: destination.appendingPathComponent("\(name).png"))
}
