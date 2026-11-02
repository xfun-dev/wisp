// Renders every Wisp icon from the glyph's path data (the same as brand/*.svg):
//   ui/mac/AppIcon.icns          macOS app icon, 16-1024 px
//   ui/win/wisp.ico              Windows exe / Start menu / window icon
//   src/assets/tray-glyph.bin    tray glyph alpha masks the client tints at runtime
// usage: swift scripts/brand.swift   (from the repo root)
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// The glyph in its 100x100 design space: the W stroke, then the curl.
func glyph(_ t: CGAffineTransform) -> (stroke: CGPath, curl: CGPath) {
    let w = CGMutablePath()
    w.move(to: CGPoint(x: 16, y: 38), transform: t)
    for (c1, c2, p) in [((22, 60), (26, 72), (33, 72)), ((41, 72), (43, 48), (50, 48)), ((57, 48), (59, 72), (67, 72)),
                        ((75, 72), (79, 57), (83, 43)), ((86, 32), (80, 23), (72, 28))] {
        w.addCurve(to: CGPoint(x: p.0, y: p.1), control1: CGPoint(x: c1.0, y: c1.1), control2: CGPoint(x: c2.0, y: c2.1), transform: t)
    }
    let curl = CGMutablePath()
    curl.move(to: CGPoint(x: 72, y: 28), transform: t)
    curl.addCurve(to: CGPoint(x: 74, y: 38), control1: CGPoint(x: 66, y: 32), control2: CGPoint(x: 67, y: 39), transform: t)
    return (w, curl)
}

func rgb(_ hex: UInt32, _ a: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat(hex >> 16 & 0xFF) / 255, green: CGFloat(hex >> 8 & 0xFF) / 255, blue: CGFloat(hex & 0xFF) / 255, alpha: a)
}

/// A top-down drawing context of `size` px (y grows downward, like the SVG).
func canvas(_ size: Int) -> CGContext {
    let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.translateBy(x: 0, y: CGFloat(size))
    ctx.scaleBy(x: 1, y: -1)
    return ctx
}

func strokeWithGradient(_ ctx: CGContext, _ path: CGPath, width: CGFloat, from: CGPoint, to: CGPoint, colors: [CGColor]) {
    ctx.saveGState()
    ctx.addPath(path)
    ctx.setLineWidth(width)
    ctx.setLineCap(.round)
    ctx.setLineJoin(.round)
    ctx.replacePathWithStrokedPath()
    ctx.clip()
    let g = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB)!, colors: colors as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(g, start: from, end: to, options: [])
    ctx.restoreGState()
}

/// The app icon on Apple's grid: an 824/1024 body with 185/1024 corners.
func appIcon(_ size: Int) -> CGImage {
    let s = CGFloat(size) / 1024
    let ctx = canvas(size)
    let body = CGRect(x: 100 * s, y: 100 * s, width: 824 * s, height: 824 * s)
    let shape = CGPath(roundedRect: body, cornerWidth: 185 * s, cornerHeight: 185 * s, transform: nil)
    ctx.saveGState()
    ctx.addPath(shape)
    ctx.clip()
    let bg = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB)!, colors: [rgb(0x1D2637), rgb(0x0A0E15)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(bg, start: CGPoint(x: 0, y: body.minY), end: CGPoint(x: 0, y: body.maxY), options: [])
    ctx.restoreGState()
    if size >= 64 {
        ctx.addPath(CGPath(roundedRect: body.insetBy(dx: 4 * s, dy: 4 * s), cornerWidth: 181 * s, cornerHeight: 181 * s, transform: nil))
        ctx.setStrokeColor(rgb(0xFFFFFF, 0.08))
        ctx.setLineWidth(8 * s)
        ctx.strokePath()
    }
    let unit = 8.24 * s
    let t = CGAffineTransform(translationX: 100 * s, y: 100 * s).scaledBy(x: unit, y: unit)
    let g = glyph(t)
    // small sizes get a bolder stroke so the W survives
    let width = (size <= 32 ? 11 : 9) * unit
    strokeWithGradient(ctx, g.stroke, width: width, from: body.origin, to: CGPoint(x: body.maxX, y: body.maxY), colors: [rgb(0xC6FFF5), rgb(0x2FD5BE)])
    if size >= 64 {
        ctx.addPath(g.curl)
        ctx.setStrokeColor(rgb(0x2FD5BE))
        ctx.setLineWidth(4.5 * unit)
        ctx.setLineCap(.round)
        ctx.strokePath()
    }
    return ctx.makeImage()!
}

/// The tray glyph as an alpha mask: the W alone, bold enough for 16 px.
func trayMask(_ size: Int) -> [UInt8] {
    var data = [UInt8](repeating: 0, count: size * size)
    data.withUnsafeMutableBytes { buf in
        // white on black in 8-bit gray: each byte is the stroke's coverage, i.e. its alpha
        let ctx = CGContext(data: buf.baseAddress, width: size, height: size, bitsPerComponent: 8, bytesPerRow: size,
                            space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)!
        ctx.translateBy(x: 0, y: CGFloat(size))
        ctx.scaleBy(x: 1, y: -1)
        // the W is wider than tall: scale it up 1.2x about its centre so it fills the square
        let unit = CGFloat(size) / 100
        let t = CGAffineTransform(scaleX: unit, y: unit).translatedBy(x: 50, y: 50).scaledBy(x: 1.2, y: 1.2).translatedBy(x: -50, y: -47.5)
        ctx.addPath(glyph(t).stroke)
        ctx.setLineWidth(11 * 1.2 * unit)
        ctx.setLineCap(.round)
        ctx.setLineJoin(.round)
        ctx.setStrokeColor(gray: 1, alpha: 1)
        ctx.strokePath()
    }
    return data
}

func png(_ image: CGImage) -> Data {
    let out = NSMutableData()
    let dest = CGImageDestinationCreateWithData(out, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, image, nil)
    CGImageDestinationFinalize(dest)
    return out as Data
}

let fm = FileManager.default

// macOS: an iconset, then iconutil
let iconset = fm.temporaryDirectory.appendingPathComponent("AppIcon.iconset")
try? fm.removeItem(at: iconset)
try fm.createDirectory(at: iconset, withIntermediateDirectories: true)
for base in [16, 32, 128, 256, 512] {
    try png(appIcon(base)).write(to: iconset.appendingPathComponent("icon_\(base)x\(base).png"))
    try png(appIcon(base * 2)).write(to: iconset.appendingPathComponent("icon_\(base)x\(base)@2x.png"))
}
let iconutil = Process()
iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
iconutil.arguments = ["-c", "icns", iconset.path, "-o", "ui/mac/AppIcon.icns"]
try iconutil.run()
iconutil.waitUntilExit()
precondition(iconutil.terminationStatus == 0, "iconutil failed")

// Windows: an .ico of PNG entries
let icoSizes = [16, 20, 24, 32, 40, 48, 64, 256]
let images = icoSizes.map { png(appIcon($0)) }
var ico = Data([0, 0, 1, 0, UInt8(icoSizes.count), 0])
var offset = 6 + 16 * icoSizes.count
func le16(_ v: Int) -> [UInt8] { [UInt8(v & 0xFF), UInt8(v >> 8 & 0xFF)] }
func le32(_ v: Int) -> [UInt8] { le16(v & 0xFFFF) + le16(v >> 16) }
for (size, img) in zip(icoSizes, images) {
    let side = UInt8(size == 256 ? 0 : size)
    ico.append(contentsOf: [side, side, 0, 0] + le16(1) + le16(32) + le32(img.count) + le32(offset))
    offset += img.count
}
images.forEach { ico.append($0) }
try ico.write(to: URL(fileURLWithPath: "ui/win/wisp.ico"))

// tray masks: for each size, one byte of side length, then side*side alpha bytes
var masks = Data()
for size in [16, 20, 24, 32, 40, 48] {
    masks.append(UInt8(size))
    masks.append(contentsOf: trayMask(size))
}
try masks.write(to: URL(fileURLWithPath: "src/assets/tray-glyph.bin"))
print("wrote ui/mac/AppIcon.icns, ui/win/wisp.ico, src/assets/tray-glyph.bin")
