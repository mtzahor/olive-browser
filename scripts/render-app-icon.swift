import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let size = 1024
let output = CommandLine.arguments.dropFirst().first ?? "assets/olive-browser.png"
let colorSpace = CGColorSpaceCreateDeviceRGB()
let bytesPerRow = size * 4
var pixels = [UInt8](repeating: 0, count: size * bytesPerRow)
guard let context = CGContext(
    data: &pixels,
    width: size,
    height: size,
    bitsPerComponent: 8,
    bytesPerRow: bytesPerRow,
    space: colorSpace,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else {
    fatalError("Could not create icon drawing context")
}

// Use a top-left origin to keep the drawing coordinates identical to the SVG source.
context.translateBy(x: 0, y: CGFloat(size))
context.scaleBy(x: 1, y: -1)
context.setShouldAntialias(true)
context.setAllowsAntialiasing(true)

func rgba(_ value: UInt32, alpha: CGFloat = 1) -> CGColor {
    let red = CGFloat((value >> 16) & 0xff) / 255
    let green = CGFloat((value >> 8) & 0xff) / 255
    let blue = CGFloat(value & 0xff) / 255
    return CGColor(colorSpace: colorSpace, components: [red, green, blue, alpha])!
}

let paper = rgba(0xF4F2E8)
let page = rgba(0xF8F7F0)
let ink = rgba(0x2F3927)
let olive = rgba(0x5C7137)
let paleOlive = rgba(0xE4EBCD)
let grid = rgba(0xB7C992)
let chromeLight = rgba(0xDCE6C9)
let accent = rgba(0xD79A57)

func fillRounded(_ rect: CGRect, radius: CGFloat, color: CGColor) {
    context.setFillColor(color)
    context.addPath(CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil))
    context.fillPath()
}

func fillEllipse(_ rect: CGRect, color: CGColor) {
    context.setFillColor(color)
    context.fillEllipse(in: rect)
}

func stroke(_ path: CGPath, color: CGColor, width: CGFloat, cap: CGLineCap = .butt) {
    context.setStrokeColor(color)
    context.setLineWidth(width)
    context.setLineCap(cap)
    context.addPath(path)
    context.strokePath()
}

// Warm paper tile and the dark frame give the icon a strong silhouette in the dock.
fillRounded(CGRect(x: 0, y: 0, width: 1024, height: 1024), radius: 220, color: paper)
fillRounded(CGRect(x: 56, y: 56, width: 912, height: 912), radius: 202, color: ink)
fillRounded(CGRect(x: 88, y: 88, width: 848, height: 848), radius: 174, color: page)

// Browser chrome: a flat-bottomed green header inside the rounded page.
let chrome = CGMutablePath()
chrome.move(to: CGPoint(x: 88, y: 262))
chrome.addLine(to: CGPoint(x: 88, y: 182))
chrome.addCurve(to: CGPoint(x: 182, y: 88), control1: CGPoint(x: 88, y: 130), control2: CGPoint(x: 130, y: 88))
chrome.addLine(to: CGPoint(x: 842, y: 88))
chrome.addCurve(to: CGPoint(x: 936, y: 182), control1: CGPoint(x: 936, y: 130), control2: CGPoint(x: 894, y: 88))
chrome.addLine(to: CGPoint(x: 936, y: 262))
chrome.closeSubpath()
context.setFillColor(olive)
context.addPath(chrome)
context.fillPath()

for x in [160, 220, 280] {
    fillEllipse(CGRect(x: x - 18, y: 158, width: 36, height: 36), color: chromeLight)
}
fillRounded(CGRect(x: 388, y: 144, width: 410, height: 64), radius: 32, color: rgba(0x809655, alpha: 0.72))
fillEllipse(CGRect(x: 419, y: 165, width: 22, height: 22), color: rgba(0xEAF0DF))
let address = CGMutablePath()
address.move(to: CGPoint(x: 477, y: 176))
address.addLine(to: CGPoint(x: 755, y: 176))
stroke(address, color: rgba(0xEAF0DF, alpha: 0.92), width: 14, cap: .round)

// Globe-like page content: a quiet discovery motif behind the leaf.
fillEllipse(CGRect(x: 264, y: 337, width: 496, height: 496), color: paleOlive)
stroke(CGPath(ellipseIn: CGRect(x: 264, y: 337, width: 496, height: 496), transform: nil), color: grid, width: 18)
let globe = CGMutablePath()
globe.move(to: CGPoint(x: 294, y: 585)); globe.addLine(to: CGPoint(x: 730, y: 585))
globe.move(to: CGPoint(x: 512, y: 337)); globe.addCurve(to: CGPoint(x: 512, y: 833), control1: CGPoint(x: 408, y: 407), control2: CGPoint(x: 408, y: 755))
globe.move(to: CGPoint(x: 512, y: 337)); globe.addCurve(to: CGPoint(x: 512, y: 833), control1: CGPoint(x: 616, y: 407), control2: CGPoint(x: 616, y: 755))
globe.move(to: CGPoint(x: 333, y: 459)); globe.addCurve(to: CGPoint(x: 691, y: 459), control1: CGPoint(x: 438, y: 350), control2: CGPoint(x: 586, y: 350))
stroke(globe, color: grid, width: 14, cap: .round)

// Single leaf silhouette. Its pale midrib keeps it readable at 16–32 px.
let leaf = CGMutablePath()
leaf.move(to: CGPoint(x: 412, y: 742))
leaf.addCurve(to: CGPoint(x: 447, y: 507), control1: CGPoint(x: 386, y: 664), control2: CGPoint(x: 399, y: 584))
leaf.addCurve(to: CGPoint(x: 670, y: 364), control1: CGPoint(x: 566, y: 390), control2: CGPoint(x: 635, y: 373))
leaf.addCurve(to: CGPoint(x: 569, y: 611), control1: CGPoint(x: 662, y: 552), control2: CGPoint(x: 628, y: 631))
leaf.addCurve(to: CGPoint(x: 412, y: 742), control1: CGPoint(x: 523, y: 657), control2: CGPoint(x: 458, y: 718))
leaf.closeSubpath()
context.setFillColor(olive)
context.addPath(leaf)
context.fillPath()

let midrib = CGMutablePath()
midrib.move(to: CGPoint(x: 411, y: 742))
midrib.addCurve(to: CGPoint(x: 692, y: 454), control1: CGPoint(x: 534, y: 586), control2: CGPoint(x: 630, y: 505))
stroke(midrib, color: paper, width: 18, cap: .round)

let stem = CGMutablePath()
stem.move(to: CGPoint(x: 410, y: 742)); stem.addCurve(to: CGPoint(x: 292, y: 800), control1: CGPoint(x: 359, y: 757), control2: CGPoint(x: 319, y: 782))
stroke(stem, color: ink, width: 20, cap: .round)
let stemTip = CGMutablePath()
stemTip.move(to: CGPoint(x: 331, y: 799)); stemTip.addLine(to: CGPoint(x: 280, y: 830))
stroke(stemTip, color: ink, width: 14, cap: .round)

fillEllipse(CGRect(x: 756, y: 738, width: 44, height: 44), color: accent)

guard let image = context.makeImage(),
      let destination = CGImageDestinationCreateWithURL(
          URL(fileURLWithPath: output) as CFURL,
          UTType.png.identifier as CFString,
          1,
          nil
      ) else {
    fatalError("Could not create PNG destination")
}
CGImageDestinationAddImage(destination, image, nil)
guard CGImageDestinationFinalize(destination) else {
    fatalError("Could not write (output)")
}
