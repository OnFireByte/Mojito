// make-icon.swift — builds the Mojito icons from the 64x64 pixel art
// (docs/icon/mojito.png).
//
// Usage: swift scripts/make-icon.swift
//
// Renders every size with nearest-neighbor sampling (no smoothing, so pixels
// stay crisp), composites it on a dark rounded-square canvas, writes
// AppIcon.iconset/, compiles Mojito.icns with iconutil, refreshes the 1024px
// docs/icon/mojito-appicon.png master, and emits the bare art as menu-bar
// sprites (app/assets/mojito-status-{16,32}.png) for the status item.

import AppKit

let root = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .deletingLastPathComponent()
let iconDir = root.appendingPathComponent("docs/icon")
let srcPath = iconDir.appendingPathComponent("mojito.png")
let iconsetPath = iconDir.appendingPathComponent("AppIcon.iconset")
let icnsPath = iconDir.appendingPathComponent("Mojito.icns")

guard let srcData = try? Data(contentsOf: srcPath),
      let src = NSBitmapImageRep(data: srcData) else {
    fatalError("cannot read \(srcPath.lastPathComponent)")
}
let srcW = src.pixelsWide
let srcH = src.pixelsHigh

// flatten source pixels once; colorAt/setColor use top-down coordinates
var colors: [NSColor] = []
colors.reserveCapacity(srcW * srcH)
for y in 0..<srcH {
    for x in 0..<srcW {
        colors.append(src.colorAt(x: x, y: y) ?? NSColor.clear)
    }
}

let artFraction: CGFloat = 832.0 / 1024.0 // art size relative to canvas
let cornerFraction: CGFloat = 232.0 / 1024.0 // rounded-square corner radius
let circleFraction: CGFloat = 396.0 / 1024.0 // backdrop circle radius
let bgColor = NSColor(red: 0x0C / 255.0, green: 0x15 / 255.0, blue: 0x11 / 255.0, alpha: 1)
let circleColor = NSColor(red: 0x17 / 255.0, green: 0x27 / 255.0, blue: 0x1E / 255.0, alpha: 1)

func render(_ size: Int) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

    let f = CGFloat(size)
    let rounded = NSBezierPath(
        roundedRect: NSRect(x: 0, y: 0, width: f, height: f),
        xRadius: f * cornerFraction, yRadius: f * cornerFraction)
    rounded.addClip()

    bgColor.setFill()
    NSBezierPath(rect: NSRect(x: 0, y: 0, width: f, height: f)).fill()

    circleColor.setFill()
    NSBezierPath(ovalIn: NSRect(
        x: f / 2 - f * circleFraction, y: f / 2 - f * circleFraction,
        width: 2 * f * circleFraction, height: 2 * f * circleFraction)
    ).fill()

    let artPx = f * artFraction
    let scale = artPx / CGFloat(srcW)
    let base = (f - artPx) / 2

    if scale >= 1 {
        // one fill per source pixel, spans snapped to whole device pixels
        // so edges stay hard even at fractional scales like 6.5x
        for sy in 0..<srcH {
            let y0 = (base + CGFloat(srcH - 1 - sy) * scale).rounded(.toNearestOrAwayFromZero)
            let y1 = (base + CGFloat(srcH - sy) * scale).rounded(.toNearestOrAwayFromZero)
            guard y1 > y0 else { continue }
            for sx in 0..<srcW {
                let c = colors[sy * srcW + sx]
                guard c.alphaComponent > 0 else { continue }
                let x0 = (base + CGFloat(sx) * scale).rounded(.toNearestOrAwayFromZero)
                let x1 = (base + CGFloat(sx + 1) * scale).rounded(.toNearestOrAwayFromZero)
                guard x1 > x0 else { continue }
                c.setFill()
                NSBezierPath(rect: NSRect(x: x0, y: y0, width: x1 - x0, height: y1 - y0)).fill()
            }
        }
    } else {
        // one fill per destination pixel, sampling the nearest source pixel
        for py in 0..<size {
            for px in 0..<size {
                let sx = Int(((CGFloat(px) + 0.5 - base) / scale).rounded(.down))
                let sy = Int(((CGFloat(py) + 0.5 - base) / scale).rounded(.down))
                guard sx >= 0, sx < srcW, sy >= 0, sy < srcH else { continue }
                let c = colors[sy * srcW + sx]
                guard c.alphaComponent > 0 else { continue }
                c.setFill()
                NSBezierPath(rect: NSRect(x: CGFloat(px), y: CGFloat(size - 1 - py), width: 1, height: 1)).fill()
            }
        }
    }

    NSGraphicsContext.restoreGraphicsState()
    return rep
}

func png(_ rep: NSBitmapImageRep) -> Data {
    rep.representation(using: .png, properties: [:])!
}

// the bare art at an exact size: transparent, no canvas, nearest-decimated —
// what the status item button draws
func sprite(_ size: Int) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    let step = CGFloat(srcW) / CGFloat(size)
    for py in 0..<size {
        for px in 0..<size {
            let sx = min(srcW - 1, Int(((CGFloat(px) + 0.5) * step).rounded(.down)))
            let sy = min(srcH - 1, Int(((CGFloat(py) + 0.5) * step).rounded(.down)))
            rep.setColor(colors[sy * srcW + sx], atX: px, y: py)
        }
    }
    return rep
}

// iconset members: (pixel size, filename)
let members: [(Int, String)] = [
    (16, "icon_16x16.png"), (32, "icon_16x16@2x.png"),
    (32, "icon_32x32.png"), (64, "icon_32x32@2x.png"),
    (128, "icon_128x128.png"), (256, "icon_128x128@2x.png"),
    (256, "icon_256x256.png"), (512, "icon_256x256@2x.png"),
    (512, "icon_512x512.png"), (1024, "icon_512x512@2x.png"),
]

try? FileManager.default.removeItem(at: iconsetPath)
try! FileManager.default.createDirectory(at: iconsetPath, withIntermediateDirectories: true)

var cache: [Int: Data] = [:]
for (size, name) in members {
    let data: Data
    if let cached = cache[size] {
        data = cached
    } else {
        data = png(render(size))
        cache[size] = data
    }
    try! data.write(to: iconsetPath.appendingPathComponent(name))
}
try! png(render(1024)).write(to: iconDir.appendingPathComponent("mojito-appicon.png"))

// menu-bar sprites for the status item (include_bytes! from mojito-macos)
let assetsDir = root.appendingPathComponent("app/assets")
try! FileManager.default.createDirectory(at: assetsDir, withIntermediateDirectories: true)
try! png(sprite(32)).write(to: assetsDir.appendingPathComponent("mojito-status-32.png"))
try! png(sprite(16)).write(to: assetsDir.appendingPathComponent("mojito-status-16.png"))

let util = Process()
util.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
util.arguments = ["-c", "icns", iconsetPath.path, "-o", icnsPath.path]
try! util.run()
util.waitUntilExit()
guard util.terminationStatus == 0 else {
    fatalError("iconutil failed with status \(util.terminationStatus)")
}

print("wrote \(icnsPath.path) from \(members.count) sizes")
