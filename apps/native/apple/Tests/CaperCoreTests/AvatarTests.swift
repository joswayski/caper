import XCTest
import SwiftUI
#if os(macOS)
import AppKit
#elseif os(iOS)
import UIKit
#endif
@testable import CaperCore

final class AvatarTests: XCTestCase {
    func testDailyIconSelectionIsStableBoundedAndDoesNotRepeat() {
        XCTAssertEqual(CaperDailyIcon.select(day: "2026-10-03", savedDay: "2026-10-03", savedIndex: 42, random: 799), 42)
        XCTAssertEqual(Set((0..<800).map { CaperDailyIcon.select(day: "new", savedDay: nil, savedIndex: nil, random: UInt64($0)) }), Set(0..<800))
        for old in [0, 42, 799] {
            let choices = Set((0..<799).map { CaperDailyIcon.select(day: "new", savedDay: "old", savedIndex: old, random: UInt64($0)) })
            XCTAssertEqual(choices, Set(0..<800).subtracting([old]))
        }
    }

    func testDailyIconUsesUTCDateBoundaries() {
        XCTAssertEqual(CaperDailyIcon.utcDay(containing: Date(timeIntervalSince1970: 86_399)), "1970-01-01")
        XCTAssertEqual(CaperDailyIcon.utcDay(containing: Date(timeIntervalSince1970: 86_400)), "1970-01-02")
    }

    func testWordmarkAndDockReuseThePersistedChoiceAcrossRollover() throws {
        let suite = "caper-daily-icon-test-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let before = Date(timeIntervalSince1970: 86_399)
        let after = Date(timeIntervalSince1970: 86_400)
        XCTAssertEqual(CaperDailyIcon.current(now: before, defaults: defaults, random: 143), 143)
        XCTAssertEqual(CaperDailyIcon.current(now: before, defaults: defaults, random: 799), 143)
        XCTAssertEqual(CaperDailyIcon.current(now: after, defaults: defaults, random: 143), 144)
        XCTAssertEqual(CaperDailyIcon.current(now: after, defaults: defaults, random: 0), 144)
    }

    func testPersistedIndicesAndFallback() throws {
        for index in [0, 31, 32, 255, 256, 799] { XCTAssertEqual(CaperAvatar.index(for: index), index) }
        for index in [nil, -1, 800] { XCTAssertNil(CaperAvatar.index(for: index)) }

        XCTAssertNil(try JSONDecoder().decode(Account.self, from: Data(#"{"id":"old"}"#.utf8)).avatarId)
        XCTAssertEqual(try JSONDecoder().decode(Account.self, from: Data(#"{"id":"saved","avatarId":16}"#.utf8)).avatarId, 16)
    }

    @MainActor
    func testAllAvatarsResolveFromTheShippedResourceBundle() {
        for index in 0..<800 {
            XCTAssertNotNil(CaperAvatar.image(for: index), "Missing bundled avatar \(index)")
        }
        for index in [nil, -1, 800] { XCTAssertNil(CaperAvatar.image(for: index)) }
    }

    @MainActor
    func testBrandingAssetsResolveWithoutPaintingTheAvatarTile() throws {
        for index in 0..<800 {
            let name = "caper-branding-\(index)"
            #if os(macOS)
            let image = try XCTUnwrap(caperResourceBundle.image(forResource: NSImage.Name(name)))
            XCTAssertFalse(image.isTemplate)
            #else
            let image = try XCTUnwrap(UIImage(named: name, in: caperResourceBundle, compatibleWith: nil))
            #endif
            guard [0, 46, 80, 537, 799].contains(index) else { continue }
            var pixels = [UInt8](repeating: 0, count: 256 * 256 * 4)
            try pixels.withUnsafeMutableBytes { bytes in
                let context = try XCTUnwrap(CGContext(data: bytes.baseAddress, width: 256, height: 256,
                    bitsPerComponent: 8, bytesPerRow: 256 * 4, space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
                #if os(macOS)
                NSGraphicsContext.saveGraphicsState()
                defer { NSGraphicsContext.restoreGraphicsState() }
                NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
                image.draw(in: CGRect(x: 0, y: 0, width: 256, height: 256))
                #else
                UIGraphicsPushContext(context)
                defer { UIGraphicsPopContext() }
                image.draw(in: CGRect(x: 0, y: 0, width: 256, height: 256))
                #endif
            }
            XCTAssertEqual(pixels[(128 * 256 + 252) * 4 + 3], 0, "Branding \(index) must not have a tile")
            XCTAssertTrue(stride(from: 3, to: pixels.count, by: 4).contains { pixels[$0] == 255 }, "Branding \(index) cannot be blank")
        }
    }

    #if os(macOS)
    @MainActor
    func testDockCharactersHaveNoTileAndFillTheCanvasWithoutClipping() throws {
        for index in [0, 22, 46, 80, 537, 799] {
            let image = try XCTUnwrap(CaperDailyIcon.dockImage(for: index))
            XCTAssertFalse(image.isTemplate)
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(image.tiffRepresentation)))
            XCTAssertEqual(bitmap.pixelsWide, 512)
            XCTAssertEqual(bitmap.pixelsHigh, 512)
            var left = 512, top = 512, right = -1, bottom = -1
            for y in 0..<512 {
                for x in 0..<512 where try XCTUnwrap(bitmap.colorAt(x: x, y: y)).alphaComponent > 0 {
                    left = min(left, x); right = max(right, x)
                    top = min(top, y); bottom = max(bottom, y)
                }
            }
            // Raw branding leaves a large canvas margin; avatar tiles fill it.
            // The fitted character instead fills 480px with a 16px clear edge.
            XCTAssertLessThanOrEqual(abs(max(right - left + 1, bottom - top + 1) - 480), 2, "Bad fit for \(index)")
            XCTAssertLessThanOrEqual(abs(left + right - 511), 2, "Off-center X for \(index)")
            XCTAssertLessThanOrEqual(abs(top + bottom - 511), 2, "Off-center Y for \(index)")
            XCTAssertGreaterThanOrEqual(left, 15)
            XCTAssertGreaterThanOrEqual(top, 15)
            XCTAssertLessThanOrEqual(right, 496)
            XCTAssertLessThanOrEqual(bottom, 496)
            for (x, y) in [(0, 0), (511, 511), (256, 4), (4, 256), (507, 256), (256, 507)] {
                XCTAssertEqual(try XCTUnwrap(bitmap.colorAt(x: x, y: y)).alphaComponent, 0)
            }
            XCTAssertLessThan(try XCTUnwrap(bitmap.colorAt(x: 24, y: 24)).alphaComponent, 0.01,
                "No circular tile behind character \(index)")
        }
        for index in [-1, 800] { XCTAssertNil(CaperDailyIcon.dockImage(for: index)) }
    }

    @MainActor
    func testDockFittingPreservesAspectColorsAndOrientationOfOffCenterArtwork() throws {
        let context = try XCTUnwrap(CGContext(data: nil, width: 512, height: 512,
            bitsPerComponent: 8, bytesPerRow: 512 * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        XCTAssertNil(CaperDailyIcon.fittedDockImage(NSImage(cgImage: try XCTUnwrap(context.makeImage()),
            size: NSSize(width: 512, height: 512))))
        // A deliberately off-center 120x60 rectangle with unequal color regions.
        context.setFillColor(CGColor(red: 0, green: 1, blue: 0, alpha: 1))
        context.fill(CGRect(x: 287, y: 41, width: 40, height: 60))
        context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
        context.fill(CGRect(x: 327, y: 41, width: 80, height: 60))
        context.setFillColor(CGColor(red: 0, green: 0, blue: 1, alpha: 1))
        context.fill(CGRect(x: 327, y: 81, width: 80, height: 20))
        let source = NSImage(cgImage: try XCTUnwrap(context.makeImage()), size: NSSize(width: 512, height: 512))
        let fitted = try XCTUnwrap(CaperDailyIcon.fittedDockImage(source))
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(fitted.tiffRepresentation)))
        // Independent geometry: 120x60 scales by 4 into 480x240, centered at 256.
        XCTAssertEqual(try XCTUnwrap(bitmap.colorAt(x: 15, y: 256)).alphaComponent, 0)
        XCTAssertEqual(try XCTUnwrap(bitmap.colorAt(x: 496, y: 256)).alphaComponent, 0)
        XCTAssertEqual(try XCTUnwrap(bitmap.colorAt(x: 256, y: 135)).alphaComponent, 0)
        XCTAssertEqual(try XCTUnwrap(bitmap.colorAt(x: 256, y: 376)).alphaComponent, 0)
        let green = try XCTUnwrap(bitmap.colorAt(x: 56, y: 256)?.usingColorSpace(.deviceRGB))
        let red = try XCTUnwrap(bitmap.colorAt(x: 455, y: 335)?.usingColorSpace(.deviceRGB))
        let blue = try XCTUnwrap(bitmap.colorAt(x: 455, y: 175)?.usingColorSpace(.deviceRGB))
        XCTAssertEqual(green.greenComponent, 1, accuracy: 0.01)
        XCTAssertEqual(red.redComponent, 1, accuracy: 0.01)
        XCTAssertEqual(blue.blueComponent, 1, accuracy: 0.01)
        XCTAssertEqual(green.alphaComponent, 1)
        XCTAssertEqual(red.alphaComponent, 1)
        XCTAssertEqual(blue.alphaComponent, 1)
    }
    #endif

    @MainActor
    func testAvatarsRenderColoredPixelsRatherThanBlankOrTemplateImages() throws {
        for index in [0, 31, 32, 799] {
            #if os(macOS) && arch(x86_64)
            // Hosted Intel Macs crash inside Metal when ImageRenderer starts.
            // Still rasterize the same shipped asset on the CPU; iOS and ARM
            // additionally exercise the complete SwiftUI Avatar view below.
            let image = try XCTUnwrap(CaperAvatar.image(for: index))
            XCTAssertFalse(image.isTemplate, "Avatar \(index) must preserve its original colors")
            #else
            let renderer = ImageRenderer(content: Avatar(name: "Test", size: 64, avatarID: index))
            let image = try XCTUnwrap(renderer.cgImage)
            #endif
            var pixels = [UInt8](repeating: 0, count: 64 * 64 * 4)
            let colors: Set<UInt32> = try pixels.withUnsafeMutableBytes { bytes in
                let context = try XCTUnwrap(CGContext(data: bytes.baseAddress, width: 64, height: 64,
                    bitsPerComponent: 8, bytesPerRow: 64 * 4, space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
                #if os(macOS) && arch(x86_64)
                NSGraphicsContext.saveGraphicsState()
                defer { NSGraphicsContext.restoreGraphicsState() }
                NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
                context.addEllipse(in: CGRect(x: 0, y: 0, width: 64, height: 64))
                context.clip()
                image.draw(in: CGRect(x: 0, y: 0, width: 64, height: 64))
                #else
                context.draw(image, in: CGRect(x: 0, y: 0, width: 64, height: 64))
                #endif
                let values = bytes.bindMemory(to: UInt8.self)
                return Set(stride(from: 0, to: values.count, by: 4).compactMap { offset in
                    guard values[offset + 3] == 255 else { return nil }
                    return UInt32(values[offset]) << 16 | UInt32(values[offset + 1]) << 8 | UInt32(values[offset + 2])
                })
            }
            XCTAssertGreaterThan(colors.count, 3, "Avatar \(index) must contain original multicolor artwork")
            XCTAssertEqual(pixels[3], 0, "The corner must remain transparent")
            XCTAssertGreaterThan(pixels[(32 * 64 + 32) * 4 + 3], 0, "The center cannot be blank")
        }
    }
}
