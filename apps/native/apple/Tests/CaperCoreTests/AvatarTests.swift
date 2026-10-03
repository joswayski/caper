import XCTest
import SwiftUI
#if os(macOS)
import AppKit
#endif
@testable import CaperCore

final class AvatarTests: XCTestCase {
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
