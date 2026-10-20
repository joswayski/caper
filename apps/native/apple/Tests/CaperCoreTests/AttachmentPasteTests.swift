import XCTest
import UniformTypeIdentifiers
@testable import CaperCore
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

@MainActor final class AttachmentPasteTests: XCTestCase {
    /// Exercise the same system-pasteboard entry point as the composer, not
    /// the representation-selection helper. Restore the user's clipboard.
    private func withPasteboard(_ representations: [(UTType, Data)], run: () throws -> Void) throws {
        #if os(iOS)
        let saved = UIPasteboard.general.items
        defer { UIPasteboard.general.items = saved }
        UIPasteboard.general.items = [Dictionary(uniqueKeysWithValues: representations.map { ($0.0.identifier, $0.1) })]
        #elseif os(macOS)
        let board = NSPasteboard.general
        let saved = (board.pasteboardItems ?? []).map { item in
            item.types.compactMap { type in item.data(forType: type).map { (type, $0) } }
        }
        defer {
            board.clearContents()
            board.writeObjects(saved.map { values in
                let item = NSPasteboardItem()
                for (type, data) in values { item.setData(data, forType: type) }
                return item
            })
        }
        let item = NSPasteboardItem()
        for (type, data) in representations {
            item.setData(data, forType: NSPasteboard.PasteboardType(type.identifier))
        }
        board.clearContents()
        XCTAssertTrue(board.writeObjects([item]))
        #endif
        try run()
    }

    func testGIFWinsOverStaticRepresentation() throws {
        let gif = Data(base64Encoded: "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7")!
        let png = Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a9S8AAAAASUVORK5CYII=")!
        // BMP sorts before GIF on iOS; PNG is advertised first on macOS.
        try withPasteboard([(.png, png), (.bmp, Data([0x42, 0x4d])), (.gif, gif)]) {
            let files = try NativeAttachmentPaste.stage()
            defer { files.forEach { AttachmentStaging.remove($0.url) } }
            XCTAssertEqual(files.count, 1)
            let file = try XCTUnwrap(files.first)
            XCTAssertEqual(file.contentType, "image/gif")
            XCTAssertEqual(try Data(contentsOf: file.url), gif)
        }
    }

    func testPDFBytesAreStaged() throws {
        let pdf = Data("%PDF-1.7\n%%EOF".utf8)
        try withPasteboard([(.pdf, pdf)]) {
            #if os(iOS)
            XCTAssertTrue(NativeAttachmentPaste.hasAttachments)
            #endif
            let files = try NativeAttachmentPaste.stage()
            defer { files.forEach { AttachmentStaging.remove($0.url) } }
            XCTAssertEqual(files.count, 1)
            let file = try XCTUnwrap(files.first)
            XCTAssertEqual(file.name, "pasted-file.pdf")
            XCTAssertEqual(file.contentType, "application/pdf")
            XCTAssertEqual(try Data(contentsOf: file.url), pdf)
        }
    }

    func testTextBasedImageWinsOverOpaqueRepresentation() throws {
        let svg = Data("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"/>".utf8)
        try withPasteboard([(.data, Data([0, 255])), (.svg, svg)]) {
            let files = try NativeAttachmentPaste.stage()
            defer { files.forEach { AttachmentStaging.remove($0.url) } }
            XCTAssertEqual(files.count, 1)
            let file = try XCTUnwrap(files.first)
            XCTAssertEqual(file.contentType, "image/svg+xml")
            XCTAssertEqual(try Data(contentsOf: file.url), svg)
        }
    }

    func testFileURLPreservesUnknownFileAndItsName() throws {
        let source = try AttachmentStaging.stage(data: Data([0, 255, 13, 42]), filename: "custom.caperunknown")
        defer { AttachmentStaging.remove(source.url) }
        try withPasteboard([(.fileURL, Data(source.url.absoluteString.utf8)), (.utf8PlainText, Data("do not paste this path".utf8))]) {
            let files = try NativeAttachmentPaste.stage()
            defer { files.forEach { AttachmentStaging.remove($0.url) } }
            XCTAssertEqual(files.count, 1)
            let file = try XCTUnwrap(files.first)
            XCTAssertEqual(file.name, "custom.caperunknown")
            XCTAssertEqual(file.contentType, "application/octet-stream")
            XCTAssertEqual(try Data(contentsOf: file.url), Data([0, 255, 13, 42]))
        }
    }

    func testTextHTMLAndWebURLsRemainTextViewPaste() throws {
        for (type, value) in [(UTType.utf8PlainText, "hello"), (.html, "<b>hello</b>"), (.url, "https://example.com")] {
            try withPasteboard([(type, Data(value.utf8))]) {
                XCTAssertTrue(try NativeAttachmentPaste.stage().isEmpty)
                #if os(iOS)
                XCTAssertFalse(NativeAttachmentPaste.hasAttachments)
                #endif
            }
        }
    }
}
