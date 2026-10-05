import AppKit
import CaperCore
import XCTest

final class DailyDockIconTests: XCTestCase {
    @MainActor
    func testFinderSyncPreservesSignedContentsAndUsesTheDockCharacter() throws {
        let (root, app) = try makeApp()
        defer { try? FileManager.default.removeItem(at: root) }
        let contents = app.appendingPathComponent("Contents")
        let files = ["Info.plist", "MacOS/IconSync", "Resources/unchanged.txt", "_CodeSignature/CodeResources"]
        let before = try files.map { try Data(contentsOf: contents.appendingPathComponent($0)) }
        let image = try XCTUnwrap(CaperDailyIcon.dockImage(for: 22))
        XCTAssertTrue(DailyDockIcon.synchronizeInstalledIcon(image, at: app))
        try run("/usr/bin/codesign", ["--verify", "--deep", "--strict", app.path])
        XCTAssertEqual(try files.map { try Data(contentsOf: contents.appendingPathComponent($0)) }, before)
        try assertFinderIcon(at: app, matches: image)

        // Simulate the next daily character; restart need not retain metadata in
        // the release archive because every launch reapplies the persisted ID.
        let next = try XCTUnwrap(CaperDailyIcon.dockImage(for: 799))
        XCTAssertTrue(DailyDockIcon.synchronizeInstalledIcon(next, at: app))
        try run("/usr/bin/codesign", ["--verify", "--deep", "--strict", app.path])
        XCTAssertEqual(try files.map { try Data(contentsOf: contents.appendingPathComponent($0)) }, before)
        try assertFinderIcon(at: app, matches: next)
    }

    @MainActor
    func testFinderSyncRemovesCustomIconWhenPostWriteValidationFails() throws {
        let (root, app) = try makeApp()
        defer { try? FileManager.default.removeItem(at: root) }
        let before = try bitmap(NSWorkspace.shared.icon(forFile: app.path))
        let image = try XCTUnwrap(CaperDailyIcon.dockImage(for: 22))
        var checks = 0
        XCTAssertFalse(DailyDockIcon.synchronizeInstalledIcon(image, at: app, signatureIsValid: { url in
            XCTAssertEqual(url, app)
            checks += 1
            return checks == 1 // Emulate an OS rejecting the newly added metadata.
        }))
        XCTAssertEqual(checks, 2)
        try run("/usr/bin/codesign", ["--verify", "--deep", "--strict", app.path])
        let after = try bitmap(NSWorkspace.shared.icon(forFile: app.path))
        XCTAssertEqual(Data(bytes: try XCTUnwrap(before.bitmapData), count: before.bytesPerRow * before.pixelsHigh),
            Data(bytes: try XCTUnwrap(after.bitmapData), count: after.bytesPerRow * after.pixelsHigh))
    }

    @MainActor
    func testFinderSyncSkipsInvalidAndReadOnlyCopiesWithoutWritingMetadata() throws {
        let (root, app) = try makeApp()
        defer { try? FileManager.default.removeItem(at: root) }
        let image = try XCTUnwrap(CaperDailyIcon.dockImage(for: 0))
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: app.path)
        XCTAssertFalse(DailyDockIcon.synchronizeInstalledIcon(image, at: app))
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: app.path)
        try Data("tampered".utf8).write(to: app.appendingPathComponent("Contents/Resources/unchanged.txt"))
        XCTAssertFalse(DailyDockIcon.synchronizeInstalledIcon(image, at: app))
        XCTAssertFalse(FileManager.default.fileExists(atPath: app.appendingPathComponent("Icon\r").path))
        XCTAssertFalse(DailyDockIcon.synchronizeInstalledIcon(image, at: root))
    }

    @MainActor
    private func assertFinderIcon(at app: URL, matches image: NSImage) throws {
        // Read Finder's icon rather than merely checking setIcon's return value.
        let expected = try bitmap(image), found = try bitmap(NSWorkspace.shared.icon(forFile: app.path))
        for (x, y) in [(256, 256), (256, 350), (100, 100)] {
            let a = try XCTUnwrap(expected.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB))
            let b = try XCTUnwrap(found.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB))
            XCTAssertEqual(a.alphaComponent, b.alphaComponent, accuracy: 0.05)
            if a.alphaComponent > 0.9 {
                XCTAssertEqual(a.redComponent, b.redComponent, accuracy: 0.05)
                XCTAssertEqual(a.greenComponent, b.greenComponent, accuracy: 0.05)
                XCTAssertEqual(a.blueComponent, b.blueComponent, accuracy: 0.05)
            }
        }
    }

    private func makeApp() throws -> (URL, URL) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("caper-icon-test-\(UUID().uuidString)")
        let app = root.appendingPathComponent("Icon Sync.app")
        do {
            try FileManager.default.createDirectory(at: app.appendingPathComponent("Contents/MacOS"), withIntermediateDirectories: true)
            try FileManager.default.createDirectory(at: app.appendingPathComponent("Contents/Resources"), withIntermediateDirectories: true)
            let plist: [String: String] = ["CFBundleIdentifier": "chat.caper.icon-test", "CFBundleExecutable": "IconSync",
                "CFBundlePackageType": "APPL", "CFBundleVersion": "1"]
            try PropertyListSerialization.data(fromPropertyList: plist, format: .xml, options: 0)
                .write(to: app.appendingPathComponent("Contents/Info.plist"))
            try FileManager.default.copyItem(at: URL(fileURLWithPath: "/usr/bin/true"),
                to: app.appendingPathComponent("Contents/MacOS/IconSync"))
            try Data("keep signed resources unchanged".utf8).write(to: app.appendingPathComponent("Contents/Resources/unchanged.txt"))
            try run("/usr/bin/codesign", ["--force", "--sign", "-", "--timestamp=none", app.path])
            try run("/usr/bin/codesign", ["--verify", "--deep", "--strict", app.path])
            return (root, app)
        } catch {
            try? FileManager.default.removeItem(at: root)
            throw error
        }
    }

    @MainActor
    private func bitmap(_ image: NSImage) throws -> NSBitmapImageRep {
        let result = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 512, pixelsHigh: 512,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 512 * 4, bitsPerPixel: 32))
        NSGraphicsContext.saveGraphicsState()
        defer { NSGraphicsContext.restoreGraphicsState() }
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: result)
        NSGraphicsContext.current?.cgContext.clear(CGRect(x: 0, y: 0, width: 512, height: 512))
        image.draw(in: CGRect(x: 0, y: 0, width: 512, height: 512))
        return result
    }

    private func run(_ executable: String, _ arguments: [String]) throws {
        let process = Process(), output = Pipe()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        process.standardOutput = output
        process.standardError = output
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw NSError(domain: "CaperIconTest", code: Int(process.terminationStatus),
                userInfo: [NSLocalizedDescriptionKey: String(decoding: data, as: UTF8.self)])
        }
    }
}
