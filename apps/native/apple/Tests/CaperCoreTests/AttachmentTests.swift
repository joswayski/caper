import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
import XCTest
@testable import CaperCore

private final class AttachmentTokenStore: TokenStore, @unchecked Sendable {
    var token: String?
    init(_ token: String?) { self.token = token }
    func load() throws -> String? { token }
    func save(_ token: String) throws { self.token = token }
    func clear() throws { token = nil }
}

/// Records every request (method, URL, headers, JSON body) and answers from `handler`.
private final class AttachmentURLProtocol: URLProtocol, @unchecked Sendable {
    struct Recorded { let method: String; let url: URL; let headers: [String: String]; let body: Data? }
    private static let lock = NSLock()
    nonisolated(unsafe) static var handler: ((URLRequest, Data?) throws -> (Int, Data))?
    nonisolated(unsafe) private static var recorded: [Recorded] = []

    static var requests: [Recorded] { lock.withLock { recorded } }
    static func reset() { lock.withLock { recorded = []; handler = nil } }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let body = Self.body(of: request)
        Self.lock.withLock {
            Self.recorded.append(Recorded(method: request.httpMethod ?? "GET", url: request.url!,
                                          headers: request.allHTTPHeaderFields ?? [:], body: body))
        }
        do {
            guard let handler = Self.handler else { throw URLError(.badURL) }
            let (status, data) = try handler(request, body)
            client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil,
                                                                  headerFields: ["content-type": "application/json"])!, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}

    private static func body(of request: URLRequest) -> Data? {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return nil }
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = stream.read(&buffer, maxLength: buffer.count)
            if count <= 0 { break }
            data.append(buffer, count: count)
        }
        return data
    }
}

private func json(_ data: Data?) -> [String: Any]? {
    data.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
}

private let futureExp = 4_102_444_800 // 2100-01-01
private let channelID = "Chan12345678"

final class AttachmentTests: XCTestCase {
    override func tearDown() {
        AttachmentURLProtocol.reset()
        super.tearDown()
    }

    private func client(token: String? = "account-secret") -> APIClient {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [AttachmentURLProtocol.self]
        return APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration),
                         tokenStore: AttachmentTokenStore(token))
    }

    @MainActor
    private func waitUntil(_ message: String = "condition", _ predicate: @escaping @MainActor () -> Bool) async {
        for _ in 0..<300 where !predicate() { try? await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(predicate(), "timed out waiting for \(message)")
    }

    // MARK: Tolerant decoding

    func testMalformedAttachmentsAreSkippedWithoutRejectingMessagesOrHistory() async throws {
        let history = """
        {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channelID)","name":"general"},"cursor":"3","hasMore":false,"messages":[
          {"id":"m1","channelId":"\(channelID)","seq":"1","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c1",
           "content":{"version":1,"type":"text","text":"","attachments":[
             {"id":"Asset0000001","kind":"image","contentType":"image/png","name":"shot.png","size":2048,"width":800,"height":600,"preview":{},
              "url":"https://cdn.caper.chat/original/Asset0000001?exp=\(futureExp)&sig=a","previewUrl":"https://cdn.caper.chat/preview/Asset0000001?exp=\(futureExp)&sig=b"},
             {"id":"Asset0000002","kind":"image","contentType":"image/png","name":"bad-size.png","size":"big"},
             {"id":"Asset0000003","kind":"sticker","contentType":"image/png","name":"unknown.png","size":1},
             {"id":"Asset0000004","kind":"file","contentType":"text/html","name":"x.html","size":1,"url":"javascript:alert(1)"},
             {"id":"Asset0000005","kind":"file","contentType":"application/pdf","name":"gone.pdf","size":10,"unavailable":true},
             {"id":"Asset0000006","kind":"video","contentType":"video/mp4","name":"neg.mp4","size":10,"width":-4},
             "not an object", null
           ]}},
          {"id":"m2","channelId":"\(channelID)","seq":"2","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c2",
           "content":{"version":1,"type":"text","text":"wrong shape","attachments":"oops"}},
          {"id":"m3","channelId":"\(channelID)","seq":"3","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c3",
           "content":{"version":1,"type":"text","text":"plain"}}
        ]}
        """
        AttachmentURLProtocol.handler = { _, _ in (200, Data(history.utf8)) }
        let page = try await client().history(channelID: channelID)
        XCTAssertEqual(page.messages.map(\.id), ["m1", "m2", "m3"], "no message or page is rejected for bad attachments")
        let attachments = try XCTUnwrap(page.messages[0].content.attachments)
        XCTAssertEqual(attachments.map(\.id), ["Asset0000001", "Asset0000005"])
        XCTAssertEqual(page.messages[0].content.text, "")
        XCTAssertTrue(attachments[0].hasPreview)
        XCTAssertEqual(attachments[0].kind, .image)
        XCTAssertEqual(attachments[0].width, 800)
        XCTAssertTrue(attachments[1].unavailable)
        XCTAssertNil(attachments[1].url)
        XCTAssertNil(page.messages[1].content.attachments, "a non-array field is ignored")
        XCTAssertNil(page.messages[2].content.attachments)

        // Gateway frames decode through the same model.
        let frame = Data(#"{"id":"m4","channelId":"c","seq":"4","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c4","content":{"version":1,"type":"text","text":"hi","attachments":[{"id":"A","kind":"audio","contentType":"audio/mpeg","name":"a.mp3","size":5,"durationMs":1200},{"id":7}]}}"#.utf8)
        let message = try JSONDecoder().decode(ChatMessage.self, from: frame)
        XCTAssertEqual(message.content.attachments?.map(\.durationMs), [1200])
    }

    func testAttachmentRoundTripsThroughEncoding() throws {
        let attachment = ChatAttachment(id: "A", kind: .video, contentType: "video/mp4", name: "v.mp4", size: 9, width: 4, height: 3,
                                        durationMs: 10, hasPreview: true, url: "https://cdn.invalid/o", previewUrl: "https://cdn.invalid/p")
        let content = ChatContent(version: 1, type: "text", text: "", attachments: [attachment])
        let decoded = try JSONDecoder().decode(ChatContent.self, from: JSONEncoder().encode(content))
        XCTAssertEqual(decoded, content)
    }

    func testAttachmentKindMatchesInlineAllowlist() {
        XCTAssertEqual(AttachmentKind(contentType: "image/webp"), .image)
        XCTAssertEqual(AttachmentKind(contentType: "IMAGE/PNG"), .image)
        XCTAssertEqual(AttachmentKind(contentType: "image/heic"), .file, "HEIC is not inline-renderable in browsers")
        XCTAssertEqual(AttachmentKind(contentType: "image/svg+xml"), .file)
        XCTAssertEqual(AttachmentKind(contentType: "video/quicktime"), .video)
        XCTAssertEqual(AttachmentKind(contentType: "audio/x-m4a"), .audio)
        XCTAssertEqual(AttachmentKind(contentType: "application/pdf"), .file)
    }

    // MARK: URL expiry and refresh

    func testSignedURLExpiryDecisions() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        func url(_ exp: Int) -> String { "https://cdn.caper.chat/original/A?exp=\(exp)&sig=x" }
        XCTAssertEqual(AttachmentURLPolicy.expiry(of: url(1_800_000_123)), Date(timeIntervalSince1970: 1_800_000_123))
        XCTAssertTrue(AttachmentURLPolicy.needsRefresh(url(1_799_999_000), now: now), "past expiry")
        XCTAssertTrue(AttachmentURLPolicy.needsRefresh(url(1_800_001_800), now: now), "expires within the hour")
        XCTAssertFalse(AttachmentURLPolicy.needsRefresh(url(1_800_007_200), now: now), "two hours left")
        XCTAssertFalse(AttachmentURLPolicy.needsRefresh("https://cdn.caper.chat/original/A?sig=x", now: now), "no exp")
        XCTAssertFalse(AttachmentURLPolicy.needsRefresh("https://cdn.caper.chat/original/A?exp=soon", now: now))
        XCTAssertFalse(AttachmentURLPolicy.needsRefresh("file:///tmp/local.png", now: now), "pending local copies never refresh")
        XCTAssertFalse(AttachmentURLPolicy.needsRefresh(nil, now: now))
        XCTAssertTrue(AttachmentURLPolicy.refreshesAfterFailure(status: 403))
        XCTAssertTrue(AttachmentURLPolicy.refreshesAfterFailure(status: 404))
        XCTAssertFalse(AttachmentURLPolicy.refreshesAfterFailure(status: 500))
    }

    private func historyWithAttachment(exp: Int) -> Data {
        Data("""
        {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channelID)","name":"general"},"cursor":"1","hasMore":false,"messages":[
          {"id":"m1","channelId":"\(channelID)","seq":"1","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c1",
           "content":{"version":1,"type":"text","text":"","attachments":[
             {"id":"Asset0000001","kind":"image","contentType":"image/png","name":"shot.png","size":2048,"preview":{},
              "url":"https://cdn.caper.chat/original/Asset0000001?exp=\(exp)&sig=old","previewUrl":"https://cdn.caper.chat/preview/Asset0000001?exp=\(exp)&sig=old"}
           ]}}
        ]}
        """.utf8)
    }

    @MainActor
    func testExpiredURLsRefreshInOneBatchAndPatchTheTimeline() async throws {
        let history = historyWithAttachment(exp: 1_000)
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/chat/session"): return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case ("GET", "/api/chat/channels/\(channelID)/messages"): return (200, history)
            case ("POST", "/api/assets/urls"):
                return (200, Data(#"{"urls":{"Asset0000001":{"url":"https://cdn.caper.chat/original/Asset0000001?exp=\#(futureExp)&sig=new","previewUrl":"https://cdn.caper.chat/preview/Asset0000001?exp=\#(futureExp)&sig=new"}}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        XCTAssertTrue(AttachmentURLPolicy.needsRefresh(chat.messages.first?.content.attachments?.first?.url))
        chat.requestFreshAttachmentURLs(ids: ["Asset0000001"])
        chat.requestFreshAttachmentURLs(ids: ["Asset0000001"])
        await waitUntil("fresh URL") { chat.messages.first?.content.attachments?.first?.url?.contains("sig=new") == true }
        let refreshes = AttachmentURLProtocol.requests.filter { $0.url.path == "/api/assets/urls" }
        XCTAssertEqual(refreshes.count, 1, "duplicate requests are batched")
        XCTAssertEqual(json(refreshes.first?.body)?["ids"] as? [String], ["Asset0000001"])
        XCTAssertEqual(refreshes.first?.headers["Authorization"] ?? refreshes.first?.headers["authorization"], "Bearer account-secret")
        XCTAssertEqual(chat.messages.first?.content.attachments?.first?.previewUrl?.contains("sig=new"), true)
        let opened = await chat.currentURL(for: try XCTUnwrap(chat.messages.first?.content.attachments?.first))
        XCTAssertEqual(opened?.query?.contains("sig=new"), true)
        XCTAssertEqual(AttachmentURLProtocol.requests.filter { $0.url.path == "/api/assets/urls" }.count, 1, "fresh URLs are reused")
        await chat.stop()
    }

    @MainActor
    func testRefreshIsAttemptedOnceWhenAttachmentIsNoLongerVisible() async throws {
        let history = historyWithAttachment(exp: 1_000)
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/chat/session"): return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case ("GET", "/api/chat/channels/\(channelID)/messages"): return (200, history)
            case ("POST", "/api/assets/urls"): return (200, Data(#"{"urls":{}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        chat.requestFreshAttachmentURLs(ids: ["Asset0000001"])
        await waitUntil("first refresh") { AttachmentURLProtocol.requests.contains { $0.url.path == "/api/assets/urls" } }
        try await Task.sleep(for: .milliseconds(100))
        // A repeated load failure for the same stale URL must not loop.
        chat.requestFreshAttachmentURLs(ids: ["Asset0000001"])
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(AttachmentURLProtocol.requests.filter { $0.url.path == "/api/assets/urls" }.count, 1)
        XCTAssertTrue(chat.messages.first?.content.attachments?.first?.url?.contains("sig=old") == true)
        await chat.stop()
    }

    // MARK: Compression decisions

    func testCompressionSettingsDecodeWithDefaults() throws {
        let decoded = try JSONDecoder().decode(AssetUsage.self, from: Data(#"{"used":5,"limit":10,"compression":{"imageQuality":80,"paletteColors":999,"previewEdge":320}}"#.utf8))
        XCTAssertEqual(decoded.compression.imageQuality, 80)
        XCTAssertEqual(decoded.compression.paletteColors, 256, "out-of-range values fall back to defaults")
        XCTAssertEqual(decoded.compression.previewEdge, 320)
        XCTAssertEqual(decoded.compression.imageMaxEdge, 4096)
        XCTAssertEqual(decoded.compression.videoMaxHeight, 1080)
        let legacy = try JSONDecoder().decode(AssetUsage.self, from: Data(#"{"used":0,"limit":10}"#.utf8))
        XCTAssertEqual(legacy.compression, AttachmentCompression())
    }

    func testStillEncodingChoosesPaletteThenLossyThenOriginal() {
        var settings = AttachmentCompression()
        XCTAssertEqual(AttachmentPolicy.stillEncoding(colorCount: 12, settings: settings), .indexedPNG)
        XCTAssertEqual(AttachmentPolicy.stillEncoding(colorCount: 256, settings: settings), .indexedPNG)
        XCTAssertEqual(AttachmentPolicy.stillEncoding(colorCount: nil, settings: settings), .lossy(quality: 0.92))
        settings.paletteColors = 0
        XCTAssertEqual(AttachmentPolicy.stillEncoding(colorCount: 2, settings: settings), .lossy(quality: 0.92), "0 disables the palette path")
        settings.paletteColors = 16
        XCTAssertEqual(AttachmentPolicy.stillEncoding(colorCount: 17, settings: settings), .lossy(quality: 0.92))
        settings.imageQuality = 100
        XCTAssertEqual(AttachmentPolicy.stillEncoding(colorCount: nil, settings: settings), AttachmentPolicy.StillEncoding.none, "100 disables lossy re-encoding")

        XCTAssertTrue(AttachmentPolicy.compressible("image/jpeg"))
        XCTAssertTrue(AttachmentPolicy.compressible("image/heic"))
        XCTAssertFalse(AttachmentPolicy.compressible("image/gif"))
        XCTAssertFalse(AttachmentPolicy.compressible("image/svg+xml"))
        XCTAssertFalse(AttachmentPolicy.compressible("image/avif"))
        XCTAssertFalse(AttachmentPolicy.compressible("video/mp4"))
    }

    func testKeepReencodedOnlyWhenTenPercentSmallerOrOriginalNotInline() {
        XCTAssertTrue(AttachmentPolicy.keepReencoded(originalType: "image/png", originalSize: 1000, encodedSize: 900))
        XCTAssertFalse(AttachmentPolicy.keepReencoded(originalType: "image/png", originalSize: 1000, encodedSize: 901))
        XCTAssertTrue(AttachmentPolicy.keepReencoded(originalType: "image/heic", originalSize: 1000, encodedSize: 5000))
        XCTAssertEqual(AttachmentPolicy.renamed("IMG_0001.HEIC", contentType: "image/jpeg"), "IMG_0001.jpg")
        XCTAssertEqual(AttachmentPolicy.renamed("Screen Shot.tiff", contentType: "image/png"), "Screen Shot.png")
        XCTAssertEqual(AttachmentPolicy.renamed("clip.mov", contentType: "video/mp4"), "clip.mp4")
        XCTAssertEqual(AttachmentPolicy.renamed("noext", contentType: "image/webp"), "noext.webp")
        XCTAssertEqual(AttachmentPolicy.renamed("notes.txt", contentType: "text/plain"), "notes.txt")
    }

    func testPreviewSizing() {
        let settings = AttachmentCompression()
        XCTAssertTrue(AttachmentPolicy.needsPreview(kind: .image, width: 641, height: 10, byteSize: 100, settings: settings))
        XCTAssertFalse(AttachmentPolicy.needsPreview(kind: .image, width: 640, height: 480, byteSize: 524_288, settings: settings))
        XCTAssertTrue(AttachmentPolicy.needsPreview(kind: .image, width: 100, height: 100, byteSize: 524_289, settings: settings))
        XCTAssertTrue(AttachmentPolicy.needsPreview(kind: .video, width: 100, height: 100, byteSize: 1, settings: settings), "videos get a poster")
        XCTAssertFalse(AttachmentPolicy.needsPreview(kind: .audio, width: nil, height: nil, byteSize: 9_999_999, settings: settings))
        XCTAssertFalse(AttachmentPolicy.needsPreview(kind: .file, width: 4000, height: 4000, byteSize: 9_999_999, settings: settings))
        var off = settings
        off.previewEdge = 0
        XCTAssertFalse(AttachmentPolicy.needsPreview(kind: .image, width: 4000, height: 4000, byteSize: 1, settings: off))

        XCTAssertTrue(AttachmentPolicy.fitWithin(width: 4000, height: 3000, edge: 640) == (640, 480))
        XCTAssertTrue(AttachmentPolicy.fitWithin(width: 300, height: 9000, edge: 640) == (21, 640))
        XCTAssertTrue(AttachmentPolicy.fitWithin(width: 200, height: 100, edge: 640) == (200, 100), "never enlarged")
        XCTAssertTrue(AttachmentPolicy.displaySize(width: 1200, height: 600)! == (360, 180))
        XCTAssertTrue(AttachmentPolicy.displaySize(width: 600, height: 1200)! == (150, 300))
        XCTAssertNil(AttachmentPolicy.displaySize(width: nil, height: 10))
    }

    func testVideoPresetKeepsShortEdgeWithinLimit() {
        XCTAssertEqual(AttachmentPolicy.videoPreset(width: 3840, height: 2160, maxHeight: 1080), "AVAssetExportPreset1920x1080")
        XCTAssertEqual(AttachmentPolicy.videoPreset(width: 1920, height: 1080, maxHeight: 720), "AVAssetExportPreset1280x720")
        XCTAssertEqual(AttachmentPolicy.videoPreset(width: 1080, height: 1920, maxHeight: 1080), "AVAssetExportPreset3840x2160",
                       "the limit bounds the short edge, so portrait 1080p is re-encoded without scaling")
        XCTAssertEqual(AttachmentPolicy.videoPreset(width: 2160, height: 3840, maxHeight: 1080), "AVAssetExportPreset1920x1080")
        XCTAssertEqual(AttachmentPolicy.videoPreset(width: 640, height: 360, maxHeight: 1080), "AVAssetExportPreset3840x2160",
                       "small videos are never enlarged, only re-encoded")
        XCTAssertNil(AttachmentPolicy.videoPreset(width: 1920, height: 1080, maxHeight: 0), "0 disables transcoding")
        XCTAssertNil(AttachmentPolicy.videoPreset(width: 1920, height: 1080, maxHeight: 200))
    }

    func testFormatBytesAndDraftLabels() {
        XCTAssertEqual(AttachmentPolicy.formatBytes(512), "512 B")
        XCTAssertEqual(AttachmentPolicy.formatBytes(1536), "1.5 KB")
        XCTAssertEqual(AttachmentPolicy.formatBytes(146_432), "143 KB")
        XCTAssertEqual(AttachmentPolicy.formatBytes(1_677_722), "1.6 MB")
        var draft = AttachmentDraft(id: "d", name: "shot.png", kind: .image, localURL: URL(fileURLWithPath: "/tmp/x"), sourceSize: 1_677_722)
        draft.progress = 0.42
        XCTAssertEqual(draft.statusLabel, "Uploading… 42%")
        draft.storedSize = 146_432
        draft.attachment = ChatAttachment(id: "A", kind: .image, contentType: "image/webp", name: "shot.webp", size: 146_432)
        XCTAssertEqual(draft.statusLabel, "1.6 MB → 143 KB")
        draft.error = "You’ve used all of your file storage."
        XCTAssertEqual(draft.statusLabel, "You’ve used all of your file storage.")
    }

    func testFileOnlyMessagesValidate() {
        XCTAssertEqual(MessageValidation.error(for: "  "), "Write a message first.")
        XCTAssertNil(MessageValidation.error(for: "", attachmentCount: 1))
        XCTAssertNotNil(MessageValidation.error(for: "", attachmentCount: 11))
    }

    // MARK: Image pipeline

    private func rgbaImage(width: Int, height: Int, pixel: (Int, Int) -> [UInt8]) -> CGImage {
        var bytes: [UInt8] = []
        for y in 0..<height { for x in 0..<width { bytes += pixel(x, y) } }
        // Premultiply for CoreGraphics.
        for offset in stride(from: 0, to: bytes.count, by: 4) {
            let alpha = Int(bytes[offset + 3])
            for channel in 0..<3 { bytes[offset + channel] = UInt8((Int(bytes[offset + channel]) * alpha + 127) / 255) }
        }
        let provider = CGDataProvider(data: Data(bytes) as CFData)!
        return CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
                       space: CGColorSpace(name: CGColorSpace.sRGB)!,
                       bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue),
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    }

    private func decodedPixels(_ data: Data) throws -> (width: Int, height: Int, rgba: [UInt8]) {
        let source = try XCTUnwrap(CGImageSourceCreateWithData(data as CFData, nil))
        let image = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
        let pixels = try XCTUnwrap(AttachmentPreparer.rgbaPixels(image))
        return (image.width, image.height, pixels)
    }

    func testIndexedPNGRoundTripsExactPixelsThroughImageIO() throws {
        let colors: [[UInt8]] = [[255, 0, 0, 255], [0, 128, 255, 255], [0, 0, 0, 0], [20, 200, 40, 255]]
        let width = 7, height = 5
        var rgba: [UInt8] = []
        for y in 0..<height { for x in 0..<width { rgba += colors[(x * 3 + y) % colors.count] } }
        let palette = try XCTUnwrap(IndexedPNG.palette(width: width, height: height, rgba: rgba))
        XCTAssertEqual(palette.colors.count, 4)
        XCTAssertEqual(palette.colors.first, 0x0000_0000, "translucent colours come first for a short tRNS chunk")
        let png = try XCTUnwrap(IndexedPNG.encode(palette))
        XCTAssertEqual(Array(png.prefix(8)), [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        XCTAssertEqual(png[24], 2, "four colours pack at two bits per pixel")
        XCTAssertEqual(png[25], 3, "colour type 3 (indexed)")
        XCTAssertNotNil(png.range(of: Data("tRNS".utf8)))

        let decoded = try decodedPixels(png)
        XCTAssertEqual(decoded.width, width)
        XCTAssertEqual(decoded.height, height)
        for pixel in 0..<(width * height) {
            let expected = Array(rgba[pixel * 4..<pixel * 4 + 4])
            let actual = Array(decoded.rgba[pixel * 4..<pixel * 4 + 4])
            if expected[3] == 0 { XCTAssertEqual(actual[3], 0, "pixel \(pixel)") } else { XCTAssertEqual(actual, expected, "pixel \(pixel)") }
        }
    }

    func testIndexedPNGEightBitAndTranslucentPalette() throws {
        let width = 30, height = 8 // 240 distinct colours
        var rgba: [UInt8] = []
        for y in 0..<height { for x in 0..<width { rgba += [UInt8(x * 6), UInt8(y * 20), 77, x == 0 ? 128 : 255] } }
        let palette = try XCTUnwrap(IndexedPNG.palette(width: width, height: height, rgba: rgba))
        XCTAssertEqual(IndexedPNG.bitDepth(colorCount: palette.colors.count), 8)
        let decoded = try decodedPixels(try XCTUnwrap(IndexedPNG.encode(palette)))
        for pixel in 0..<(width * height) {
            for channel in 0..<4 {
                let expected = Int(rgba[pixel * 4 + channel]), actual = Int(decoded.rgba[pixel * 4 + channel])
                // Half-transparent colours pass through premultiplied drawing; allow rounding.
                XCTAssertLessThanOrEqual(abs(expected - actual), rgba[pixel * 4 + 3] == 255 ? 0 : 2, "pixel \(pixel) channel \(channel)")
            }
        }
        var many: [UInt8] = []
        for value in 0..<257 { many += [UInt8(value % 256), UInt8(value / 256), 0, 255] }
        XCTAssertNil(IndexedPNG.palette(width: 257, height: 1, rgba: many), "more than 256 colours")
        XCTAssertNil(IndexedPNG.palette(width: 2, height: 1, rgba: [0, 0, 0, 255, 1, 1, 1, 255], maxColors: 1))
    }

    func testChecksumsMatchKnownVectors() {
        XCTAssertEqual(IndexedPNG.adler32(Array("Wikipedia".utf8)), 0x11E6_0398)
        XCTAssertEqual(IndexedPNG.crc32(Array("IEND".utf8)), 0xAE42_6082)
        let large = [UInt8](repeating: 0xFF, count: 100_000)
        var a: UInt32 = 1, b: UInt32 = 0
        for byte in large { a = (a + UInt32(byte)) % 65521; b = (b + a) % 65521 }
        XCTAssertEqual(IndexedPNG.adler32(large), b << 16 | a)
    }

    func testStillEncoderUsesPaletteForFlatImagesAndLossyForPhotos() throws {
        let flat = rgbaImage(width: 64, height: 32) { x, _ in x < 32 ? [12, 13, 15, 255] : [182, 77, 50, 255] }
        let screenshot = try XCTUnwrap(AttachmentPreparer.encodeStill(flat, originalType: "image/png", settings: AttachmentCompression()))
        XCTAssertEqual(screenshot.contentType, "image/png")
        let decoded = try decodedPixels(screenshot.data)
        XCTAssertEqual(Array(decoded.rgba.prefix(4)), [12, 13, 15, 255])
        XCTAssertEqual(Array(decoded.rgba[(40 * 4)..<(40 * 4 + 4)]), [182, 77, 50, 255])

        let photo = rgbaImage(width: 64, height: 64) { x, y in [UInt8(x * 4), UInt8(y * 4), UInt8((x * y) % 256), 255] }
        let lossy = try XCTUnwrap(AttachmentPreparer.encodeStill(photo, originalType: "image/png", settings: AttachmentCompression()))
        XCTAssertTrue(["image/webp", "image/jpeg"].contains(lossy.contentType), lossy.contentType)
        XCTAssertNotEqual(lossy.contentType, "image/heic")

        var lossless = AttachmentCompression()
        lossless.imageQuality = 100
        XCTAssertNil(AttachmentPreparer.encodeStill(photo, originalType: "image/png", settings: lossless))
        XCTAssertNotNil(AttachmentPreparer.encodeStill(photo, originalType: "image/heic", settings: lossless), "HEIC always converts")
    }

    func testPrepareImageConvertsFlatJPEGToIndexedPNGWithPreview() async throws {
        let image = rgbaImage(width: 900, height: 500) { _, _ in [99, 122, 67, 255] }
        let jpeg = try XCTUnwrap(AttachmentPreparer.encode(image, type: UTType.jpeg.identifier, quality: 1))
        let original = try AttachmentStaging.stage(data: jpeg, filename: "screen.jpg")
        defer { AttachmentStaging.remove(original.url) }
        XCTAssertEqual(original.contentType, "image/jpeg")
        let prepared = await AttachmentPreparer.prepare(original, settings: AttachmentCompression())
        defer { AttachmentStaging.remove(prepared.fileURL) }
        XCTAssertEqual(prepared.contentType, "image/png")
        XCTAssertEqual(prepared.name, "screen.png")
        XCTAssertEqual(prepared.kind, .image)
        XCTAssertEqual(prepared.width, 900)
        XCTAssertEqual(prepared.height, 500)
        XCTAssertEqual(prepared.sourceSize, jpeg.count)
        XCTAssertLessThan(prepared.byteSize, jpeg.count)
        XCTAssertEqual((try prepared.fileURL.resourceValues(forKeys: [.fileSizeKey])).fileSize, prepared.byteSize)
        let preview = try XCTUnwrap(prepared.preview, "900 px exceeds the 640 px preview edge")
        XCTAssertTrue(["image/webp", "image/jpeg"].contains(preview.contentType))
        XCTAssertLessThanOrEqual(preview.data.count, 512 * 1024)
        let previewPixels = try decodedPixels(preview.data)
        XCTAssertEqual(max(previewPixels.width, previewPixels.height), 640)

        var capped = AttachmentCompression()
        capped.imageMaxEdge = 300
        let scaled = await AttachmentPreparer.prepare(original, settings: capped)
        defer { AttachmentStaging.remove(scaled.fileURL) }
        XCTAssertEqual(scaled.width, 300)
        XCTAssertTrue((166...167).contains(scaled.height ?? 0), "\(String(describing: scaled.height))")
    }

    func testNonMediaFilesUploadUnchanged() async throws {
        let original = try AttachmentStaging.stage(data: Data("hello".utf8), filename: "notes.txt")
        defer { AttachmentStaging.remove(original.url) }
        let prepared = await AttachmentPreparer.prepare(original, settings: AttachmentCompression())
        XCTAssertEqual(prepared.fileURL, original.url)
        XCTAssertEqual(prepared.contentType, "text/plain")
        XCTAssertEqual(prepared.kind, .file)
        XCTAssertEqual(prepared.byteSize, 5)
        XCTAssertNil(prepared.preview)
        XCTAssertEqual(AttachmentStaging.contentType(forFilename: "x.unknownext"), "application/octet-stream")
    }

    // MARK: Upload sequence

    func testUploadReservesPutsExactHeadersWithoutCredentialsThenCompletes() async throws {
        let original = try AttachmentStaging.stage(data: Data(repeating: 7, count: 2048), filename: "shot.png")
        defer { AttachmentStaging.remove(original.url) }
        var prepared = PreparedAttachment(original: original)
        prepared.sourceSize = 9000
        prepared.width = 800
        prepared.height = 600
        prepared.preview = AttachmentPreviewImage(data: Data(repeating: 1, count: 100), contentType: "image/jpeg")
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.absoluteString ?? "") {
            case ("POST", "https://caper.invalid/api/assets"):
                return (201, Data(#"{"id":"Asset0000001","kind":"image","upload":{"method":"PUT","url":"https://r2.invalid/original/Asset0000001?X-Amz-Signature=o","headers":{"content-type":"image/png","content-disposition":"attachment; filename=\"shot.png\""}},"previewUpload":{"method":"PUT","url":"https://r2.invalid/preview/Asset0000001?X-Amz-Signature=p","headers":{"content-type":"image/jpeg"}},"storage":{"used":2148,"limit":10000}}"#.utf8))
            case ("PUT", _): return (200, Data())
            case ("POST", "https://caper.invalid/api/assets/Asset0000001/complete"):
                return (200, Data(#"{"id":"Asset0000001","kind":"image","contentType":"image/png","name":"shot.png","size":2048,"width":800,"height":600,"preview":{}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let fractions = FractionLog()
        let attachment = try await AttachmentUploader.upload(prepared, channelID: channelID, api: client()) { fractions.append($0) }
        XCTAssertEqual(attachment.id, "Asset0000001")
        XCTAssertTrue(attachment.hasPreview)

        let requests = AttachmentURLProtocol.requests
        XCTAssertEqual(requests.map { "\($0.method) \($0.url.host ?? "")\($0.url.path)" }, [
            "POST caper.invalid/api/assets",
            "PUT r2.invalid/preview/Asset0000001",
            "PUT r2.invalid/original/Asset0000001",
            "POST caper.invalid/api/assets/Asset0000001/complete",
        ])
        let body = try XCTUnwrap(json(requests[0].body))
        XCTAssertEqual(body["channelId"] as? String, channelID)
        XCTAssertEqual(body["filename"] as? String, "shot.png")
        XCTAssertEqual(body["contentType"] as? String, "image/png")
        XCTAssertEqual(body["byteSize"] as? Int, 2048)
        XCTAssertEqual(body["sourceByteSize"] as? Int, 9000)
        XCTAssertEqual(body["width"] as? Int, 800)
        XCTAssertNil(body["durationMs"], "absent values are omitted, not null")
        XCTAssertEqual((body["preview"] as? [String: Any])?["byteSize"] as? Int, 100)
        XCTAssertEqual((body["preview"] as? [String: Any])?["contentType"] as? String, "image/jpeg")
        func header(_ request: AttachmentURLProtocol.Recorded, _ name: String) -> String? {
            request.headers.first { $0.key.lowercased() == name }?.value
        }
        XCTAssertEqual(header(requests[0], "authorization"), "Bearer account-secret")
        XCTAssertEqual(header(requests[1], "content-type"), "image/jpeg")
        XCTAssertNil(header(requests[1], "authorization"), "storage PUTs never carry the account credential")
        XCTAssertEqual(header(requests[2], "content-type"), "image/png")
        XCTAssertEqual(header(requests[2], "content-disposition"), "attachment; filename=\"shot.png\"")
        XCTAssertNil(header(requests[2], "authorization"))
        XCTAssertEqual(header(requests[3], "authorization"), "Bearer account-secret")
        XCTAssertEqual(fractions.values.last, 1)
    }

    func testStorageFullAndStorageRefusalErrorsAreExplained() async throws {
        let original = try AttachmentStaging.stage(data: Data(repeating: 1, count: 10), filename: "a.bin")
        defer { AttachmentStaging.remove(original.url) }
        AttachmentURLProtocol.handler = { _, _ in (413, Data(#"{"error":"storage limit reached","code":"storage_full"}"#.utf8)) }
        do {
            _ = try await AttachmentUploader.upload(PreparedAttachment(original: original), channelID: channelID, api: client()) { _ in }
            XCTFail("expected storage_full")
        } catch {
            XCTAssertEqual(AttachmentPolicy.uploadErrorMessage(error), "You’ve used all of your file storage.")
        }
        XCTAssertEqual(AttachmentURLProtocol.requests.count, 1, "nothing is uploaded after a refused reservation")

        AttachmentURLProtocol.reset()
        AttachmentURLProtocol.handler = { request, _ in
            if request.httpMethod == "PUT" { return (403, Data()) }
            return (201, Data(#"{"id":"Asset0000002","upload":{"method":"PUT","url":"https://r2.invalid/o","headers":{"content-type":"application/octet-stream"}}}"#.utf8))
        }
        do {
            _ = try await AttachmentUploader.upload(PreparedAttachment(original: original), channelID: channelID, api: client()) { _ in }
            XCTFail("expected storage refusal")
        } catch {
            XCTAssertEqual(AttachmentPolicy.uploadErrorMessage(error), "Storage refused the upload.")
        }
        XCTAssertFalse(AttachmentURLProtocol.requests.contains { $0.url.path.hasSuffix("/complete") })
        XCTAssertEqual(AttachmentPolicy.uploadErrorMessage(APIError(status: 429, message: "x")), "Uploading too quickly. Try again shortly.")
    }

    @MainActor
    func testComposerUploadsThenSendsFileOnlyMessageWithAttachmentIDs() async throws {
        AttachmentURLProtocol.handler = { request, body in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/chat/session"): return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case ("GET", "/api/chat/channels/\(channelID)/messages"):
                return (200, Data(#"{"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\#(channelID)","name":"general"},"messages":[],"cursor":"0","hasMore":false}"#.utf8))
            case ("GET", "/api/assets/usage"):
                return (200, Data(#"{"used":0,"limit":1000000,"compression":{"imageQuality":92,"imageMaxEdge":4096,"paletteColors":256,"previewEdge":640,"videoMaxHeight":1080,"videoBitrateKbps":4000,"audioBitrateKbps":128}}"#.utf8))
            case ("POST", "/api/assets"):
                return (201, Data(#"{"id":"Asset0000009","kind":"file","upload":{"method":"PUT","url":"https://r2.invalid/original/Asset0000009","headers":{"content-type":"text/plain","content-disposition":"attachment; filename=\"notes.txt\""}}}"#.utf8))
            case ("PUT", _): return (200, Data())
            case ("POST", "/api/assets/Asset0000009/complete"):
                return (200, Data(#"{"id":"Asset0000009","kind":"file","contentType":"text/plain","name":"notes.txt","size":5}"#.utf8))
            case ("POST", "/api/chat/channels/\(channelID)/messages"):
                let input = json(body) ?? [:]
                let response: [String: Any] = [
                    "id": "Message0000001", "channelId": channelID, "seq": "1", "createdAt": "2026-10-03T00:00:00Z",
                    "clientMessageId": input["clientMessageId"] as? String ?? "", "author": ["id": "self", "name": "Me", "isGuest": false],
                    "content": ["version": 1, "type": "text", "text": input["text"] as? String ?? "",
                                "attachments": [["id": "Asset0000009", "kind": "file", "contentType": "text/plain", "name": "notes.txt", "size": 5,
                                                 "url": "https://cdn.caper.chat/original/Asset0000009?exp=\(futureExp)&sig=s"]]],
                ]
                return (200, try JSONSerialization.data(withJSONObject: response))
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        XCTAssertFalse(chat.canAttach, "hidden until usage succeeds")
        await chat.checkUploadAvailability()
        XCTAssertTrue(chat.canAttach)
        XCTAssertFalse(chat.canSubmit, "nothing to send yet")

        let file = try AttachmentStaging.stage(data: Data("hello".utf8), filename: "notes.txt")
        chat.addAttachments([file])
        XCTAssertEqual(chat.attachmentDrafts.count, 1)
        await waitUntil("upload") { chat.attachmentDrafts.first?.attachment != nil }
        XCTAssertNil(chat.attachmentDrafts.first?.error)
        XCTAssertTrue(chat.canSubmit, "a file-only message can be sent")

        await chat.send()
        XCTAssertNil(chat.error)
        XCTAssertTrue(chat.attachmentDrafts.isEmpty)
        let send = try XCTUnwrap(AttachmentURLProtocol.requests.last { $0.method == "POST" && $0.url.path == "/api/chat/channels/\(channelID)/messages" })
        let body = try XCTUnwrap(json(send.body))
        XCTAssertEqual(body["attachmentIds"] as? [String], ["Asset0000009"])
        XCTAssertEqual(body["text"] as? String, "")
        XCTAssertEqual(chat.messages.first?.content.attachments?.first?.id, "Asset0000009")
        await chat.stop()
    }

    func testTextOnlySendOmitsAttachmentIDs() async throws {
        AttachmentURLProtocol.handler = { _, body in
            let input = json(body) ?? [:]
            let response: [String: Any] = [
                "id": "Message0000001", "channelId": channelID, "seq": "1", "createdAt": "now",
                "clientMessageId": input["clientMessageId"] as? String ?? "", "author": ["id": "self", "name": "Me", "isGuest": false],
                "content": ["version": 1, "type": "text", "text": "hi"],
            ]
            return (200, try JSONSerialization.data(withJSONObject: response))
        }
        _ = try await client().send(channelID: channelID, sessionToken: "chat", clientMessageID: "c", text: "hi")
        let body = try XCTUnwrap(json(AttachmentURLProtocol.requests.first?.body))
        XCTAssertNil(body["attachmentIds"], "the key is included only when non-empty")
    }
}

private final class FractionLog: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: [Double] = []
    func append(_ value: Double) { lock.withLock { stored.append(value) } }
    var values: [Double] { lock.withLock { stored } }
}
