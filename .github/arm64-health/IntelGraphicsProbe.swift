// Disposable native rendering control, never linked into Shortcut Radar.
import AppKit
import SwiftUI

@main
struct IntelGraphicsProbe {
    @MainActor
    static func main() throws {
        let mode = CommandLine.arguments[1]
        let destination = URL(fileURLWithPath: CommandLine.arguments[2])
        _ = NSApplication.shared
        print("begin: \(mode)")
        fflush(stdout)
        let symbol = "exclamationmark.octagon.fill"
        let image: CGImage?
        switch mode {
        case "appkit":
            let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 144,
                pixelsHigh: 144, bitsPerSample: 8, samplesPerPixel: 4,
                hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                bytesPerRow: 0, bitsPerPixel: 0)!
            let context = NSGraphicsContext(bitmapImageRep: bitmap)!
            let artwork = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)!
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = context
            let scale = min(132 / artwork.size.width, 132 / artwork.size.height)
            let size = NSSize(width: artwork.size.width * scale, height: artwork.size.height * scale)
            artwork.draw(in: NSRect(x: (144 - size.width) / 2, y: (144 - size.height) / 2,
                                   width: size.width, height: size.height),
                         from: .zero, operation: .sourceOver, fraction: 1)
            NSGraphicsContext.restoreGraphicsState()
            image = bitmap.cgImage
        case "swiftui-image", "swiftui-context":
            let renderer = ImageRenderer(content: Image(systemName: symbol).resizable().scaledToFit()
                .symbolRenderingMode(.monochrome).foregroundStyle(Color.blue)
                .frame(width: 44, height: 44).padding(2))
            renderer.scale = 3
            if mode == "swiftui-image" {
                image = renderer.cgImage
            } else {
                let context = CGContext(data: nil, width: 144, height: 144,
                    bitsPerComponent: 8, bytesPerRow: 144 * 4,
                    space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
                context.scaleBy(x: 3, y: 3)
                renderer.render(rasterizationScale: 3) { _, render in render(context) }
                image = context.makeImage()
            }
        case "hosting", "hosting-canvas", "hosting-blur":
            let content: AnyView
            if mode == "hosting-canvas" {
                content = AnyView(Canvas { context, size in
                    context.fill(Path(ellipseIn: CGRect(origin: .zero, size: size)), with: .color(.green))
                })
            } else if mode == "hosting-blur" {
                content = AnyView(Circle().fill(.green).blur(radius: 1))
            } else { content = AnyView(Text("Intel graphics control").padding()) }
            let view = NSHostingView(rootView: content)
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 320, height: 100),
                                  styleMask: [.titled], backing: .buffered, defer: false)
            window.contentView = view
            window.orderFront(nil)
            RunLoop.main.run(until: Date().addingTimeInterval(1))
            window.orderOut(nil)
            print("hosting survived")
            return
        default:
            fatalError("Unknown probe mode")
        }
        guard let image,
              let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
        else { throw NSError(domain: "IntelGraphicsProbe", code: 1) }
        try png.write(to: destination)
        print("rendered: \(image.width)x\(image.height), \(png.count) bytes")
    }
}
