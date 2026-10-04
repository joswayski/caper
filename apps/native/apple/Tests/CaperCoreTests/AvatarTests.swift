import XCTest
import SwiftUI
#if os(macOS)
import AppKit
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
