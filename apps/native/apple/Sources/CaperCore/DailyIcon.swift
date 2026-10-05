import Foundation
#if os(macOS)
import AppKit
#endif

/// Shared selection for in-app wordmarks and the running macOS Dock icon.
/// Account avatar IDs are deliberately unrelated to this installation-local choice.
public enum CaperDailyIcon {
    public static let count = 800

    public static func current(now: Date = Date(), defaults: UserDefaults = .standard,
                               random: UInt64 = UInt64.random(in: UInt64.min...UInt64.max)) -> Int {
        // Retain the existing macOS keys; iOS uses them only for in-app branding.
        let day = utcDay(containing: now)
        let savedDay = defaults.string(forKey: "daily-dock-icon-day-v1")
        let savedIndex = defaults.object(forKey: "daily-dock-icon-index-v1") as? Int
        let index = select(day: day, savedDay: savedDay, savedIndex: savedIndex, random: random)
        if day != savedDay || index != savedIndex {
            defaults.set(day, forKey: "daily-dock-icon-day-v1")
            defaults.set(index, forKey: "daily-dock-icon-index-v1")
        }
        return index
    }

    public static func utcDay(containing date: Date) -> String {
        let formatter = DateFormatter()
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter.string(from: date)
    }

    public static func select(day: String, savedDay: String?, savedIndex: Int?, random: UInt64) -> Int {
        if day == savedDay, let savedIndex, (0..<count).contains(savedIndex) { return savedIndex }
        if let savedIndex, (0..<count).contains(savedIndex) {
            let candidate = Int(random % UInt64(count - 1))
            return candidate >= savedIndex ? candidate + 1 : candidate
        }
        return Int(random % UInt64(count))
    }

    #if os(macOS)
    @MainActor
    public static func dockImage(for index: Int) -> NSImage? {
        guard (0..<count).contains(index),
              let character = caperResourceBundle.image(forResource: NSImage.Name("caper-branding-\(index)")) else { return nil }
        return fittedDockImage(character)
    }

    /// Fit the visible character, not the avatar's original canvas. Keep only a
    /// small transparent edge so hats/accessories aren't clipped by the Dock.
    @MainActor
    static func fittedDockImage(_ character: NSImage) -> NSImage? {
        let side = 512
        guard let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
        var pixels = [UInt8](repeating: 0, count: side * side * 4)
        let cropped: CGImage? = pixels.withUnsafeMutableBytes { bytes in
            guard let context = CGContext(data: bytes.baseAddress, width: side, height: side,
                bitsPerComponent: 8, bytesPerRow: side * 4, space: colorSpace,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return nil }
            NSGraphicsContext.saveGraphicsState()
            defer { NSGraphicsContext.restoreGraphicsState() }
            NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
            character.draw(in: CGRect(x: 0, y: 0, width: CGFloat(side), height: CGFloat(side)))
            let values = bytes.bindMemory(to: UInt8.self)
            var left = side, top = side, right = -1, bottom = -1
            for y in 0..<side {
                for x in 0..<side where values[(y * side + x) * 4 + 3] != 0 {
                    left = min(left, x); right = max(right, x)
                    top = min(top, y); bottom = max(bottom, y)
                }
            }
            guard right >= left, bottom >= top else { return nil }
            return context.makeImage()?.cropping(to: CGRect(x: CGFloat(left), y: CGFloat(top),
                width: CGFloat(right - left + 1), height: CGFloat(bottom - top + 1)))
        }
        guard let cropped,
              let context = CGContext(data: nil, width: side, height: side,
                bitsPerComponent: 8, bytesPerRow: side * 4, space: colorSpace,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return nil }
        let scale = 480 / CGFloat(max(cropped.width, cropped.height))
        let width = CGFloat(cropped.width) * scale, height = CGFloat(cropped.height) * scale
        context.interpolationQuality = .high
        context.draw(cropped, in: CGRect(x: (512 - width) / 2, y: (512 - height) / 2,
            width: width, height: height))
        guard let result = context.makeImage() else { return nil }
        return NSImage(cgImage: result, size: NSSize(width: CGFloat(side), height: CGFloat(side)))
    }
    #endif
}
