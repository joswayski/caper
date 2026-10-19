import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
import XCTest
@testable import CaperCore

private final class PictureTokenStore: TokenStore, @unchecked Sendable {
    var token: String?
    init(_ token: String?) { self.token = token }
    func load() throws -> String? { token }
    func save(_ token: String) throws { self.token = token }
    func clear() throws { token = nil }
}

/// Records every request (method, URL, headers, body) and answers from `handler`.
private final class PictureURLProtocol: URLProtocol, @unchecked Sendable {
    struct Recorded { let method: String; let url: URL; let headers: [String: String]; let body: Data? }
    private static let lock = NSLock()
    nonisolated(unsafe) private static var storedHandler: ((URLRequest) throws -> (Int, Data))?
    nonisolated(unsafe) private static var recorded: [Recorded] = []

    static var handler: ((URLRequest) throws -> (Int, Data))? {
        get { lock.withLock { storedHandler } }
        set { lock.withLock { storedHandler = newValue } }
    }
    static var requests: [Recorded] { lock.withLock { recorded } }
    static func reset() { lock.withLock { recorded = []; storedHandler = nil } }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let body = Self.body(of: request)
        Self.lock.withLock {
            Self.recorded.append(Recorded(method: request.httpMethod ?? "GET", url: request.url!, headers: request.allHTTPHeaderFields ?? [:], body: body))
        }
        do {
            guard let handler = Self.handler else { throw URLError(.badURL) }
            let (status, data) = try handler(request)
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

private final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    func next() -> Int {
        lock.lock()
        defer { lock.unlock() }
        value += 1
        return value
    }
}

private let pictureID = "Pic00000000000000001"
private let pictureURL = "https://cdn.caper.chat/assets/\(pictureID)?exp=4102444800&sig=abc"
private let spaceID = "Space1234567"

private func reservation(contentType: String, filename: String) -> Data {
    Data(#"{"id":"\#(pictureID)","kind":"image","upload":{"method":"PUT","url":"https://r2.invalid/original/\#(pictureID)","headers":{"content-type":"\#(contentType)","content-disposition":"inline; filename=\"\#(filename)\""}}}"#.utf8)
}

private func json(_ data: Data?) -> [String: Any]? {
    data.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
}

final class ProfilePictureTests: XCTestCase {
    override func setUp() {
        super.setUp()
        PictureURLProtocol.reset()
    }

    override func tearDown() {
        PictureURLProtocol.reset()
        super.tearDown()
    }

    private func client() -> APIClient {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [PictureURLProtocol.self]
        return APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration),
                         tokenStore: PictureTokenStore("account-secret"))
    }

    private func header(_ request: PictureURLProtocol.Recorded, _ name: String) -> String? {
        request.headers.first { $0.key.lowercased() == name }?.value
    }

    // MARK: Wire fields

    func testPictureFieldsDecodeWherePeopleAndSpacesAppearAndOldServersOmitThem() throws {
        let decoder = JSONDecoder()
        let picture = #""avatarAsset":"\#(pictureID)","avatarUrl":"\#(pictureURL)""#
        let account = try decoder.decode(Account.self, from: Data(#"{"id":"self","avatarId":4,\#(picture)}"#.utf8))
        XCTAssertEqual(account.avatarPicture?.asset, pictureID)
        XCTAssertEqual(account.avatarPicture?.url.absoluteString, pictureURL)
        XCTAssertEqual(account.avatarId, 4)
        XCTAssertNil(try decoder.decode(Account.self, from: Data(#"{"id":"old","avatarId":4}"#.utf8)).avatarPicture, "old servers: the mascot")
        XCTAssertNil(try decoder.decode(Account.self, from: Data(#"{"id":"a","avatarAsset":"\#(pictureID)"}"#.utf8)).avatarPicture,
                     "no CDN signing means no URL: keep the mascot")
        XCTAssertNil(CaperPicture(asset: pictureID, url: "file:///etc/passwd"))
        XCTAssertNil(CaperPicture(asset: "", url: pictureURL))

        XCTAssertEqual(try decoder.decode(Member.self, from: Data(#"{"id":"m","username":"m","displayName":"M","owner":false,\#(picture)}"#.utf8)).avatarPicture?.asset, pictureID)
        XCTAssertEqual(try decoder.decode(ChatAuthor.self, from: Data(#"{"id":"a","name":"A","isGuest":false,\#(picture)}"#.utf8)).avatarPicture?.asset, pictureID)
        XCTAssertEqual(try decoder.decode(Person.self, from: Data(#"{"id":"p","username":"p","displayName":"P",\#(picture)}"#.utf8)).avatarPicture?.asset, pictureID)
        XCTAssertEqual(try decoder.decode(DirectMessagePeer.self, from: Data(#"{"id":"p","username":"p","displayName":"P",\#(picture)}"#.utf8)).avatarPicture?.asset, pictureID)
        XCTAssertEqual(try decoder.decode(ReactorPerson.self, from: Data(#"{"id":"r","username":null,"displayName":null,"avatarId":null,\#(picture)}"#.utf8)).avatarPicture?.asset, pictureID)
        XCTAssertEqual(try decoder.decode(VoiceSpectator.self, from: Data(#"{"id":"v","name":"V","muted":false,"deafened":false,\#(picture)}"#.utf8)).avatarPicture?.asset, pictureID)
        XCTAssertEqual(try decoder.decode(VoiceParticipant.self, from: Data(#"{"id":"v","name":"V","muted":false,"deafened":false,"tracks":[],\#(picture)}"#.utf8)).avatarPicture?.asset, pictureID)
        XCTAssertEqual(try decoder.decode(BlockedAccount.self, from: Data(#"{"id":"b","username":"b","displayName":"B",\#(picture)}"#.utf8)).avatarPicture?.asset, pictureID)
        XCTAssertEqual(try decoder.decode(FeedbackReport.Reporter.self, from: Data(#"{"id":"f","username":"f","displayName":"F",\#(picture)}"#.utf8)).avatarPicture?.asset, pictureID)

        let icon = #""iconAsset":"\#(pictureID)","iconUrl":"\#(pictureURL)""#
        let spaces = try decoder.decode(SpacesResponse.self, from: Data(#"""
        {"spaces":[{"id":"\#(spaceID)","name":"Studio","ownerId":"o",\#(icon)},{"id":"Other1234567","name":"Plain","ownerId":"o"},
                   {"id":"Demo12345678","name":"General","ownerId":"o","demo":true,\#(icon)}],
         "invitations":[{"id":"Invite123456","name":"Invited","ownerId":"o",\#(icon)}],
         "limits":{"ownedSpaces":20,"totalSpaces":100,"channelsPerSpace":50}}
        """#.utf8))
        XCTAssertEqual(spaces.spaces.map { $0.iconPicture?.asset }, [pictureID, nil, nil], "the demo space keeps its initial")
        XCTAssertEqual(spaces.invitations.first?.iconPicture?.asset, pictureID)

        let preview = try decoder.decode(InvitePreview.self, from: Data(#"""
        {"code":"aB3dE5gH9k","space":{"name":"Studio",\#(icon)},"channel":null,
         "inviter":{"username":"owner","displayName":"Owner","avatarId":2,\#(picture)},"memberCount":3,"expiresAt":null}
        """#.utf8))
        XCTAssertEqual(preview.space.iconPicture?.asset, pictureID)
        XCTAssertEqual(preview.inviter.avatarPicture?.asset, pictureID)
        let oldPreview = try decoder.decode(InvitePreview.self, from: Data(#"""
        {"code":"aB3dE5gH9k","space":{"name":"Studio"},"channel":null,"inviter":{"username":"owner","displayName":"Owner"},"memberCount":3,"expiresAt":null}
        """#.utf8))
        XCTAssertNil(oldPreview.space.iconPicture)
        XCTAssertNil(oldPreview.inviter.avatarPicture)
    }

    func testSuggestionsAndProfileCardsCarryPictures() throws {
        let member = Member(id: "member000001", username: "maya", displayName: "Maya", owner: false, avatarId: 31,
                            avatarAsset: pictureID, avatarUrl: pictureURL)
        XCTAssertEqual(MentionCandidate(member).picture?.asset, pictureID)
        let card = MentionCard.resolve(MentionPill(id: member.id, username: member.username), members: [member], people: nil, peers: [],
                                       account: nil, viewerID: "self")
        XCTAssertEqual(card.picture?.asset, pictureID)
        XCTAssertEqual(card.avatarId, 31)
        let author = ChatAuthor(id: "stranger0001", name: "Stranger", isGuest: false, avatarId: 2, avatarAsset: pictureID, avatarUrl: pictureURL)
        XCTAssertEqual(MentionCard.resolve(author, members: [], people: nil, peers: [], account: nil, viewerID: "self").picture?.asset, pictureID,
                       "an unknown author keeps the picture their message carried")
        XCTAssertEqual(BlockTarget(author: author).avatarAsset, pictureID)
    }

    // MARK: GIF rules

    func testGIFsAreDetectedByMagicBytesAndHeaderSize() {
        func header(_ magic: String, width: UInt16, height: UInt16) -> Data {
            Data(magic.utf8) + Data([UInt8(width & 0xFF), UInt8(width >> 8), UInt8(height & 0xFF), UInt8(height >> 8)]) + Data(count: 16)
        }
        XCTAssertTrue(ProfilePicturePolicy.isGIF(header("GIF89a", width: 1, height: 1)))
        XCTAssertTrue(ProfilePicturePolicy.isGIF(header("GIF87a", width: 1, height: 1)))
        XCTAssertFalse(ProfilePicturePolicy.isGIF(Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])), "PNG")
        XCTAssertFalse(ProfilePicturePolicy.isGIF(Data("GIF8".utf8)))
        XCTAssertNil(ProfilePicturePolicy.gifSize(Data("GIF89a".utf8)), "too short for a logical screen")

        let square = header("GIF89a", width: 1024, height: 1024)
        XCTAssertEqual(ProfilePicturePolicy.gifSize(square)?.width, 1024, "little-endian bytes 6-7")
        XCTAssertEqual(ProfilePicturePolicy.gifSize(header("GIF89a", width: 480, height: 270))?.height, 270)
        XCTAssertTrue(ProfilePicturePolicy.gifUploadsUnchanged(square))
        XCTAssertFalse(ProfilePicturePolicy.gifUploadsUnchanged(header("GIF89a", width: 1025, height: 10)))
        XCTAssertFalse(ProfilePicturePolicy.gifUploadsUnchanged(header("GIF89a", width: 10, height: 1025)))
        XCTAssertFalse(ProfilePicturePolicy.gifUploadsUnchanged(header("GIF89a", width: 0, height: 10)))
        XCTAssertFalse(ProfilePicturePolicy.gifUploadsUnchanged(square + Data(count: ProfilePicturePolicy.gifMaxBytes)), "over 5 MiB")
        XCTAssertEqual(ProfilePicturePolicy.gifMaxBytes, 5_242_880)
        XCTAssertEqual(ProfilePicturePolicy.stillMaxBytes, 1_048_576)
        // A slice that does not start at index 0 reads the same.
        let sliced = (Data([1, 2]) + square).dropFirst(2)
        XCTAssertEqual(ProfilePicturePolicy.gifSize(sliced)?.width, 1024)
    }

    // MARK: Crop and decode sizes

    func testStillsCropToACentredSquareOfAtMost512() {
        XCTAssertEqual(ProfilePicturePolicy.squareCrop(width: 4000, height: 3000), CGRect(x: 500, y: 0, width: 3000, height: 3000))
        XCTAssertEqual(ProfilePicturePolicy.squareCrop(width: 300, height: 800), CGRect(x: 0, y: 250, width: 300, height: 300))
        XCTAssertEqual(ProfilePicturePolicy.squareCrop(width: 5, height: 2), CGRect(x: 1, y: 0, width: 2, height: 2))
        XCTAssertEqual(ProfilePicturePolicy.outputEdge(width: 4000, height: 3000), 512)
        XCTAssertEqual(ProfilePicturePolicy.outputEdge(width: 300, height: 800), 300, "small pictures are never enlarged")
        XCTAssertEqual(ProfilePicturePolicy.outputEdge(width: 0, height: 0), 1)
        // Decode just enough that the short edge still covers 512 px.
        XCTAssertEqual(ProfilePicturePolicy.decodeMaxPixelSize(width: 4000, height: 3000), 683)
        XCTAssertEqual(ProfilePicturePolicy.decodeMaxPixelSize(width: 3000, height: 4000), 683)
        XCTAssertEqual(ProfilePicturePolicy.decodeMaxPixelSize(width: 600, height: 600), 512)
        XCTAssertEqual(ProfilePicturePolicy.decodeMaxPixelSize(width: 300, height: 800), 800)
        XCTAssertEqual(ProfilePicturePolicy.decodeMaxPixelSize(width: 20_000, height: 1_000), 4_096, "panoramas stay bounded")
    }

    func testDisplayDecodingIsNearDisplaySizeAndCoversTheSquare() {
        XCTAssertEqual(ProfilePicturePolicy.displayPixelSize(points: 34, scale: 3), 128)
        XCTAssertEqual(ProfilePicturePolicy.displayPixelSize(points: 30, scale: 2), 64)
        XCTAssertEqual(ProfilePicturePolicy.displayPixelSize(points: 16, scale: 1), 64)
        XCTAssertEqual(ProfilePicturePolicy.displayPixelSize(points: 48, scale: 3), 192)
        XCTAssertEqual(ProfilePicturePolicy.displayPixelSize(points: 72, scale: 3), 256)
        XCTAssertEqual(ProfilePicturePolicy.displayPixelSize(points: 34, scale: 4), 128, "at most 3x")
        XCTAssertEqual(ProfilePicturePolicy.displayPixelSize(points: 200, scale: 3), 256, "at most 256 px")
        XCTAssertEqual(ProfilePicturePolicy.coverPixelSize(display: 128, width: 480, height: 480), 128)
        XCTAssertEqual(ProfilePicturePolicy.coverPixelSize(display: 64, width: 400, height: 200), 128)
        XCTAssertEqual(ProfilePicturePolicy.coverPixelSize(display: 128, width: 1000, height: 100), 256, "within twice the display size")
        XCTAssertEqual(ProfilePicturePolicy.coverPixelSize(display: 128, width: 64, height: 64), 64, "never enlarged")
        XCTAssertEqual(ProfilePicturePolicy.coverPixelSize(display: 128, width: 0, height: 0), 128)
    }

    func testFailedPicturesRetryWithAFreshURLOrLater() {
        let failed = URL(string: "https://cdn.caper.chat/assets/a?exp=1&sig=old")!
        let fresh = URL(string: "https://cdn.caper.chat/assets/a?exp=2&sig=new")!
        let at = Date(timeIntervalSince1970: 1_000)
        XCTAssertFalse(ProfilePicturePolicy.retries(failedURL: failed, failedAt: at, url: failed, now: at.addingTimeInterval(5)))
        XCTAssertTrue(ProfilePicturePolicy.retries(failedURL: failed, failedAt: at, url: fresh, now: at.addingTimeInterval(5)))
        XCTAssertTrue(ProfilePicturePolicy.retries(failedURL: failed, failedAt: at, url: failed, now: at.addingTimeInterval(121)))
    }

    func testErrorsReadAsSentences() {
        func message(_ error: Error, _ purpose: PicturePurpose = .avatar) -> String { ProfilePicturePolicy.message(for: error, purpose: purpose) }
        XCTAssertEqual(message(PictureError.unreadable), "That picture couldn’t be used. Try another one.")
        XCTAssertEqual(message(APIError(status: 422, message: "x", code: "picture_invalid")), "That picture couldn’t be used. Try another one.")
        XCTAssertEqual(message(APIError(status: 413, message: "x", code: "picture_too_large")),
                       "That picture is too large. Try another one.")
        XCTAssertEqual(message(APIError(status: 400, message: "x", code: "picture_type")), "Use a PNG, JPEG, WebP or GIF picture.")
        XCTAssertEqual(message(APIError(status: 400, message: "invalid dimensions")), "Pictures can be up to 1024 × 1024.")
        XCTAssertEqual(message(APIError(status: 429, message: "x", code: "picture_rate_limited")),
                       "You’ve changed pictures too many times today. Try again tomorrow.")
        XCTAssertEqual(message(APIError(status: 404, message: "x"), .spaceIcon), "Only the space owner can change its icon.")
        XCTAssertEqual(message(APIError(status: 409, message: "upload not finished")), "The upload didn’t finish. Try again.")
        XCTAssertEqual(message(APIError(status: 503, message: "x")), "Picture uploads aren’t available right now.")
        XCTAssertTrue(ProfilePicturePolicy.uploadsUnavailable(APIError(status: 503, message: "x")))
        XCTAssertFalse(ProfilePicturePolicy.uploadsUnavailable(APIError(status: 500, message: "x")))
        XCTAssertEqual(message(StorageUploadError(status: 403)), "Storage refused the upload. Try again.")
        XCTAssertEqual(message(URLError(.notConnectedToInternet)), "Couldn’t reach Caper. Check your connection.")
    }

    // MARK: Preparation (real ImageIO and libwebp)

    func testGIFsUploadUnchangedWithinTheirLimitsAndShrinkBeyondThem() throws {
        let gif = try animatedGIF(width: 40, height: 20, frames: 2)
        let prepared = try ProfilePicturePreparer.prepare(gif)
        XCTAssertEqual(prepared.data, gif, "the animation is never re-encoded")
        XCTAssertEqual(prepared.contentType, "image/gif")
        XCTAssertEqual(prepared.width, 40)
        XCTAssertEqual(prepared.height, 20)
        XCTAssertEqual(prepared.filename(for: .avatar), "avatar.gif")
        XCTAssertEqual(prepared.filename(for: .spaceIcon), "space-icon.gif")

        // Over 1024 px: shrunk to a 512 px square, every frame and its timing kept.
        let wide = try animatedGIF(width: 1100, height: 700, frames: 3)
        let shrunk = try ProfilePicturePreparer.prepare(wide)
        XCTAssertEqual([shrunk.contentType, "\(shrunk.width)x\(shrunk.height)"], ["image/gif", "512x512"])
        XCTAssertLessThanOrEqual(shrunk.data.count, ProfilePicturePolicy.gifMaxBytes)
        let source = try XCTUnwrap(CGImageSourceCreateWithData(shrunk.data as CFData, nil))
        XCTAssertEqual(CGImageSourceGetCount(source), 3)
        XCTAssertEqual(GIFShrink.frameDelay(source, at: 1), 0.1, accuracy: 0.001)
        // Over 5 MiB: shrunk too (to its 20 px short edge), never refused.
        let heavy = try ProfilePicturePreparer.prepare(gif + Data(count: ProfilePicturePolicy.gifMaxBytes))
        XCTAssertEqual([heavy.width, heavy.height], [20, 20])
        XCTAssertThrowsError(try ProfilePicturePreparer.prepare(Data("not a picture".utf8))) { XCTAssertEqual($0 as? PictureError, .unreadable) }
    }

    func testStillsApplyOrientationCropToASquareAndDropLocation() throws {
        // Stored 300 x 200 with a red left and blue right half; EXIF 6 shows
        // it 200 x 300 with red on top.
        let stored = image(width: 300, height: 200) { x, _ in x < 150 ? [255, 0, 0] : [0, 0, 255] }
        let gps: [CFString: Any] = [kCGImagePropertyGPSLatitude: 51.5, kCGImagePropertyGPSLatitudeRef: "N",
                                    kCGImagePropertyGPSLongitude: 0.12, kCGImagePropertyGPSLongitudeRef: "W"]
        let jpeg = try encoded(stored, type: .jpeg, properties: [kCGImagePropertyOrientation: 6, kCGImagePropertyGPSDictionary: gps,
                                                                 kCGImageDestinationLossyCompressionQuality: 0.95])
        XCTAssertNotNil(try properties(jpeg)[kCGImagePropertyGPSDictionary], "the fixture carries a location")

        let prepared = try ProfilePicturePreparer.prepare(jpeg)
        XCTAssertEqual(prepared.contentType, "image/webp", "the bundled libwebp encodes lossy WebP")
        XCTAssertEqual(prepared.width, 200)
        XCTAssertEqual(prepared.height, 200)
        XCTAssertEqual(prepared.filename(for: .avatar), "avatar.webp")
        XCTAssertLessThanOrEqual(prepared.data.count, ProfilePicturePolicy.stillMaxBytes)
        let output = try properties(prepared.data)
        XCTAssertNil(output[kCGImagePropertyGPSDictionary], "re-encoding drops the location")
        XCTAssertEqual((output[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1, 1)
        let source = try XCTUnwrap(CGImageSourceCreateWithData(prepared.data as CFData, nil))
        let decoded = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
        XCTAssertEqual(decoded.width, 200)
        XCTAssertEqual(decoded.height, 200)
        let pixels = try XCTUnwrap(AttachmentPreparer.rgbaPixels(decoded)).straight
        func pixel(x: Int, y: Int) -> [UInt8] { Array(pixels[(y * 200 + x) * 4 ..< (y * 200 + x) * 4 + 3]) }
        XCTAssertGreaterThan(pixel(x: 100, y: 10)[0], pixel(x: 100, y: 10)[2], "the stored left (red) half is on top")
        XCTAssertGreaterThan(pixel(x: 100, y: 190)[2], pixel(x: 100, y: 190)[0], "and the right (blue) half below")
    }

    func testLargeStillsScaleTo512AndFitTheLimit() throws {
        let photo = image(width: 800, height: 600) { x, y in [UInt8(x % 256), UInt8(y % 256), UInt8((x + y) % 256)] }
        let prepared = try ProfilePicturePreparer.prepare(try encoded(photo, type: .png))
        XCTAssertEqual(prepared.width, 512)
        XCTAssertEqual(prepared.height, 512)
        XCTAssertNotEqual(prepared.contentType, "image/heic")
        XCTAssertTrue(["image/webp", "image/png", "image/jpeg"].contains(prepared.contentType))
        XCTAssertLessThanOrEqual(prepared.data.count, ProfilePicturePolicy.stillMaxBytes)
        let input = PictureCreateInput(picture: prepared, purpose: .avatar, spaceID: spaceID)
        XCTAssertNil(input.spaceId, "only space icons name a space")
        XCTAssertEqual(input.byteSize, prepared.data.count)
    }

    // MARK: Display decoding

    func testDisplayDecodingAnimatesWithinBudgetAndFallsBackToOneStill() throws {
        let gif = try animatedGIF(width: 400, height: 200, frames: 3)
        let frames = try ProfilePictureDecoder.decode(gif, pixelSize: 64)
        XCTAssertTrue(frames.animated)
        XCTAssertEqual(frames.images.count, 3)
        XCTAssertEqual(frames.durations.count, 3)
        XCTAssertEqual(frames.images.first?.width, 128, "aspect-fill: the short edge covers 64 px")
        XCTAssertEqual(frames.images.first?.height, 64)
        XCTAssertEqual(frames.cost, frames.images.reduce(0) { $0 + $1.bytesPerRow * $1.height })

        let source = try XCTUnwrap(CGImageSourceCreateWithData(gif as CFData, nil))
        XCTAssertEqual(try AttachmentImageLoader.decodeAnimation(source, maxPixelSize: 64, maxFrames: 10, byteLimit: .max)?.images.count, 3)
        XCTAssertNil(try AttachmentImageLoader.decodeAnimation(source, maxPixelSize: 64, maxFrames: 2, byteLimit: .max), "too many frames")
        // Over the budget frames shrink until they fit; when nothing fits, a still.
        let shrunk = try XCTUnwrap(AttachmentImageLoader.decodeAnimation(source, maxPixelSize: 64, maxFrames: 10, byteLimit: 3 * 32 * 32 * 4))
        XCTAssertEqual(shrunk.images.count, 3)
        XCTAssertLessThan(shrunk.images[0].width, 64)
        XCTAssertNil(try AttachmentImageLoader.decodeAnimation(source, maxPixelSize: 64, maxFrames: 10, byteLimit: 1), "nothing fits")
        XCTAssertNotNil(AttachmentImageLoader.decodeStill(source, maxPixelSize: 64))

        let still = try ProfilePictureDecoder.decode(try encoded(image(width: 30, height: 30) { _, _ in [0, 255, 0] }, type: .png), pixelSize: 64)
        XCTAssertFalse(still.animated)
        XCTAssertTrue(still.durations.isEmpty)
        XCTAssertEqual(still.images.first?.width, 30)
    }

    @MainActor
    func testLoaderCachesByAssetNotURLAndRetriesAFailureWithAFreshURL() async throws {
        let gif = try animatedGIF(width: 40, height: 40, frames: 2)
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("caper-pictures-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let first = folder.appendingPathComponent("first.gif")
        try gif.write(to: first)

        let loader = ProfilePictureLoader()
        let loaded = await loader.load(asset: "AssetA", url: first, pixelSize: 64)
        XCTAssertEqual(loaded?.images.count, 2)
        // The same asset under another (re-signed) URL comes from the cache.
        let resigned = await loader.load(asset: "AssetA", url: folder.appendingPathComponent("missing.gif"), pixelSize: 64)
        XCTAssertTrue(resigned === loaded)
        XCTAssertTrue(loader.cached(ProfilePictureLoader.key(asset: "AssetA", pixelSize: 64)) === loaded)
        // Another size decodes the downloaded file again without fetching it.
        let smaller = await loader.load(asset: "AssetA", url: folder.appendingPathComponent("missing.gif"), pixelSize: 32)
        XCTAssertEqual(smaller?.images.count, 2)
        XCTAssertFalse(smaller === loaded)

        let late = folder.appendingPathComponent("late.gif")
        let failed = await loader.load(asset: "AssetB", url: late, pixelSize: 64)
        XCTAssertNil(failed)
        try gif.write(to: late)
        let sameURL = await loader.load(asset: "AssetB", url: late, pixelSize: 64)
        XCTAssertNil(sameURL, "a failed URL is not fetched again right away")
        let fresh = await loader.load(asset: "AssetB", url: first, pixelSize: 64)
        XCTAssertNotNil(fresh, "a fresh URL for the same asset loads")
    }

    // MARK: Upload

    func testAvatarUploadPutsExactHeadersRetriesCompleteAndReturnsTheAccount() async throws {
        let completes = Counter()
        PictureURLProtocol.handler = { request in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/assets"): return (201, reservation(contentType: "image/webp", filename: "avatar.webp"))
            case ("PUT", _): return (200, Data())
            case ("POST", "/api/assets/\(pictureID)/complete"):
                if completes.next() == 1 { return (409, Data(#"{"error":"upload not finished"}"#.utf8)) }
                return (200, Data(#"{"id":"\#(pictureID)","kind":"image","account":{"id":"self","username":"me","displayName":"Me","avatarId":4,"avatarAsset":"\#(pictureID)","avatarUrl":"\#(pictureURL)"}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let picture = PreparedPicture(data: Data(repeating: 7, count: 2048), contentType: "image/webp", width: 512, height: 512)
        let account = try await ProfilePictureUploader.uploadAvatar(picture, api: client(), completeRetryDelays: [.zero])
        XCTAssertEqual(account.avatarPicture?.asset, pictureID)
        XCTAssertEqual(account.avatarId, 4)

        let requests = PictureURLProtocol.requests
        XCTAssertEqual(requests.map { "\($0.method) \($0.url.host ?? "")\($0.url.path)" }, [
            "POST caper.invalid/api/assets",
            "PUT r2.invalid/original/\(pictureID)",
            "POST caper.invalid/api/assets/\(pictureID)/complete",
            "POST caper.invalid/api/assets/\(pictureID)/complete",
        ])
        let body = try XCTUnwrap(json(requests[0].body))
        XCTAssertEqual(Set(body.keys), ["purpose", "filename", "contentType", "byteSize", "width", "height"],
                       "no channel, preview, duration, source size or space")
        XCTAssertEqual(body["purpose"] as? String, "avatar")
        XCTAssertEqual(body["filename"] as? String, "avatar.webp")
        XCTAssertEqual(body["contentType"] as? String, "image/webp")
        XCTAssertEqual(body["byteSize"] as? Int, 2048)
        XCTAssertEqual(body["width"] as? Int, 512)
        XCTAssertEqual(header(requests[1], "content-type"), "image/webp")
        XCTAssertEqual(header(requests[1], "content-disposition"), "inline; filename=\"avatar.webp\"")
        XCTAssertNil(header(requests[1], "authorization"), "storage PUTs never carry the account credential")
        XCTAssertEqual(header(requests[0], "authorization"), "Bearer account-secret")
    }

    func testSpaceIconUploadNamesTheSpaceAndReturnsIt() async throws {
        PictureURLProtocol.handler = { request in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/assets"): return (201, reservation(contentType: "image/gif", filename: "space-icon.gif"))
            case ("PUT", _): return (200, Data())
            case ("POST", "/api/assets/\(pictureID)/complete"):
                return (200, Data(#"{"id":"\#(pictureID)","kind":"image","space":{"id":"\#(spaceID)","name":"Studio","ownerId":"self","iconAsset":"\#(pictureID)","iconUrl":"\#(pictureURL)"}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let picture = PreparedPicture(data: Data("GIF89a".utf8) + Data([40, 0, 20, 0]), contentType: "image/gif", width: 40, height: 20)
        let space = try await ProfilePictureUploader.uploadSpaceIcon(picture, spaceID: spaceID, api: client(), completeRetryDelays: [])
        XCTAssertEqual(space.iconPicture?.asset, pictureID)
        let body = try XCTUnwrap(json(PictureURLProtocol.requests.first?.body))
        XCTAssertEqual(body["purpose"] as? String, "spaceIcon")
        XCTAssertEqual(body["spaceId"] as? String, spaceID)
        XCTAssertEqual(body["filename"] as? String, "space-icon.gif")
        XCTAssertEqual(body["height"] as? Int, 20)
    }

    func testAFailedUploadCancelsItsReservationAndExplainsItself() async throws {
        PictureURLProtocol.handler = { request in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/assets"): return (201, reservation(contentType: "image/webp", filename: "avatar.webp"))
            case ("PUT", _): return (403, Data())
            case ("POST", "/api/assets/\(pictureID)/cancel"): return (200, Data("{}".utf8))
            default: throw URLError(.badURL)
            }
        }
        let picture = PreparedPicture(data: Data(repeating: 1, count: 8), contentType: "image/webp", width: 8, height: 8)
        do {
            _ = try await ProfilePictureUploader.uploadAvatar(picture, api: client())
            XCTFail("expected a storage refusal")
        } catch {
            XCTAssertEqual(ProfilePicturePolicy.message(for: error, purpose: .avatar), "Storage refused the upload. Try again.")
        }
        let paths = PictureURLProtocol.requests.map { "\($0.method) \($0.url.path)" }
        XCTAssertTrue(paths.contains("POST /api/assets/\(pictureID)/cancel"))
        XCTAssertFalse(paths.contains { $0.hasSuffix("/complete") })

        PictureURLProtocol.reset()
        PictureURLProtocol.handler = { _ in (429, Data(#"{"error":"too many picture changes today; try again tomorrow","code":"picture_rate_limited"}"#.utf8)) }
        do {
            _ = try await ProfilePictureUploader.uploadAvatar(picture, api: client())
            XCTFail("expected the daily limit")
        } catch {
            XCTAssertEqual((error as? APIError)?.code, "picture_rate_limited")
        }
        XCTAssertEqual(PictureURLProtocol.requests.count, 1, "nothing is uploaded after a refused reservation")
    }

    // MARK: Own profile refresh

    @MainActor
    func testHistoryPaginationAndStaleSelectionKeepCurrentIdentity() async throws {
        PictureURLProtocol.handler = { request in
            if request.httpMethod == "POST" { return (409, Data(#"{"error":"stale avatar"}"#.utf8)) }
            if request.url?.path == "/api/account/me" {
                return (200, Data(#"{"id":"self","username":"me","avatarId":4}"#.utf8))
            }
            if request.url?.query != nil { return (200, Data(#"{"pictures":[],"nextCursor":null}"#.utf8)) }
            return (200, Data(#"{"pictures":[{"avatarAsset":"\#(pictureID)","avatarUrl":"\#(pictureURL)"}],"nextCursor":"\#(pictureID)"}"#.utf8))
        }
        let model = AppModel(api: client())
        model.account = Account(id: "self", username: "me", avatarId: 4, avatarAsset: pictureID, avatarUrl: pictureURL)
        let page = try await model.avatarHistory(cursor: nil)
        let last = try await model.avatarHistory(cursor: page.nextCursor)
        XCTAssertTrue(last.pictures.isEmpty)
        XCTAssertNil(last.nextCursor)
        XCTAssertEqual(PictureURLProtocol.requests.last?.url.query, "cursor=\(pictureID)")
        do {
            try await model.selectAvatar(pictureID, expected: pictureID)
            XCTFail("stale selection must fail")
        } catch { XCTAssertEqual((error as? APIError)?.status, 409) }
        XCTAssertNil(model.account?.avatarAsset, "stale selection refreshes the current identity before reopening")
        let request = try XCTUnwrap(PictureURLProtocol.requests.first(where: { $0.method == "POST" }))
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(request.body)) as? [String: Any])
        XCTAssertEqual(body["avatarAsset"] as? String, pictureID)
        XCTAssertEqual(body["expectedAvatarAsset"] as? String, pictureID)
    }

    @MainActor
    func testOwnPictureChangesRefreshRetainedIdentitiesAndRemovalRestoresTheSameMascot() async throws {
        let account = #"{"id":"self","username":"me","displayName":"Me","avatarId":4"#
        PictureURLProtocol.handler = { request in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/assets"): return (201, reservation(contentType: "image/webp", filename: "avatar.webp"))
            case ("PUT", _): return (200, Data())
            case ("POST", "/api/assets/\(pictureID)/complete"):
                return (200, Data(#"{"id":"\#(pictureID)","kind":"image","account":\#(account),"avatarAsset":"\#(pictureID)","avatarUrl":"\#(pictureURL)"}}"#.utf8))
            case ("DELETE", "/api/account/avatar"): return (200, Data((account + "}").utf8))
            case ("GET", "/api/account/avatar/history"):
                return (200, Data(#"{"pictures":[{"avatarAsset":"\#(pictureID)","avatarUrl":"\#(pictureURL)"}],"nextCursor":null}"#.utf8))
            case ("POST", "/api/account/avatar/select"):
                return (200, Data((account + ",\"avatarAsset\":\"\(pictureID)\",\"avatarUrl\":\"\(pictureURL)\"}").utf8))
            default: throw URLError(.badURL)
            }
        }
        let model = AppModel(api: client())
        model.account = Account(id: "self", username: "me", displayName: "Me", avatarId: 4)
        model.phase = .ready
        let otherURL = "https://cdn.caper.chat/assets/Pic000000000000other?exp=1&sig=o"
        let own = Member(id: "self", username: "me", displayName: "Me", owner: true, avatarId: 4)
        let other = Member(id: "other", username: "other", displayName: "Other", owner: false, avatarId: 9,
                           avatarAsset: "Pic000000000000other", avatarUrl: otherURL)
        model.detail = SpaceDetail(space: Space(id: spaceID, name: "Space", ownerId: "self"), channels: [], members: [own, other])
        let me = ChatAuthor(id: "self", name: "Me", isGuest: false, avatarId: 4)
        let mine = ChatMessage(id: "m1", channelId: "Design123456", seq: "1", author: me,
                               content: ChatContent(version: 1, type: "text", text: "hi"), createdAt: "now", clientMessageId: "c1")
        var reply = ChatMessage(id: "m2", channelId: "Design123456", seq: "2", author: ChatAuthor(id: "other", name: "Other", isGuest: false),
                                content: ChatContent(version: 1, type: "text", text: "hello"), createdAt: "now", clientMessageId: "c2")
        reply.replyTo = MessageReply(message: mine)
        model.chat.messages = [mine, reply]

        let picture = PreparedPicture(data: Data(repeating: 1, count: 16), contentType: "image/webp", width: 512, height: 512)
        try await model.changeAvatar(picture)
        XCTAssertEqual(model.account?.avatarPicture?.asset, pictureID)
        XCTAssertEqual(model.account?.avatarId, 4, "the saved mascot never changes")
        XCTAssertEqual(model.detail?.members.map { $0.avatarPicture?.asset }, [pictureID, "Pic000000000000other"])
        XCTAssertEqual(model.chat.messages.first?.author.avatarPicture?.asset, pictureID)
        XCTAssertEqual(model.chat.messages.last?.replyTo?.author.avatarPicture?.asset, pictureID)
        XCTAssertNil(model.chat.messages.last?.author.avatarPicture, "other people are untouched")
        let card = MentionCard.resolve(MentionPill(id: "self", username: "me"), members: model.detail?.members ?? [], people: nil, peers: [],
                                       account: model.account, viewerID: "self")
        XCTAssertEqual(card.picture?.asset, pictureID)

        // A retained snapshot's newer signature for the same picture is kept.
        let resigned = "https://cdn.caper.chat/assets/\(pictureID)?exp=4102444900&sig=new"
        model.detail = SpaceDetail(space: Space(id: spaceID, name: "Space", ownerId: "self"), channels: [],
                                   members: [Member(id: "self", username: "me", displayName: "Me", owner: true, avatarId: 4,
                                                    avatarAsset: pictureID, avatarUrl: resigned)])
        XCTAssertEqual(model.detail?.members.first?.avatarUrl, resigned)

        try await model.removeAvatar()
        XCTAssertNil(model.account?.avatarAsset)
        XCTAssertEqual(model.account?.avatarId, 4, "removing restores the same mascot")
        XCTAssertNil(model.detail?.members.first?.avatarPicture, "a stale snapshot cannot bring the picture back")
        XCTAssertEqual(model.detail?.members.first?.avatarId, 4)
        XCTAssertNil(model.chat.messages.first?.author.avatarPicture)
        XCTAssertNil(model.chat.messages.last?.replyTo?.author.avatarPicture)
        let page = try await model.avatarHistory(cursor: nil)
        XCTAssertEqual(page.pictures.first?.avatarAsset, pictureID)
        XCTAssertNil(page.nextCursor)
        XCTAssertNil(model.account?.avatarAsset, "browsing history does not apply a picture")
        try await model.selectAvatar(pictureID, expected: nil)
        XCTAssertEqual(model.account?.avatarAsset, pictureID)
        XCTAssertEqual(model.chat.messages.first?.author.avatarPicture?.asset, pictureID)
        let restored = try XCTUnwrap(PictureURLProtocol.requests.last)
        XCTAssertEqual(restored.url.path, "/api/account/avatar/select")
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(restored.body)) as? [String: Any])
        XCTAssertEqual(body["avatarAsset"] as? String, pictureID)
        XCTAssertTrue(body["expectedAvatarAsset"] is NSNull)
    }

    @MainActor
    func testSpaceIconChangesUpdateTheOpenSpaceAndTheRail() async throws {
        PictureURLProtocol.handler = { request in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/assets"): return (201, reservation(contentType: "image/webp", filename: "space-icon.webp"))
            case ("PUT", _): return (200, Data())
            case ("POST", "/api/assets/\(pictureID)/complete"):
                return (200, Data(#"{"id":"\#(pictureID)","kind":"image","space":{"id":"\#(spaceID)","name":"Studio","ownerId":"self","iconAsset":"\#(pictureID)","iconUrl":"\#(pictureURL)"}}"#.utf8))
            case ("DELETE", "/api/spaces/\(spaceID)/icon"):
                return (200, Data(#"{"id":"\#(spaceID)","name":"Studio","ownerId":"self"}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let model = AppModel(api: client())
        model.account = Account(id: "self", username: "me", displayName: "Me")
        let studio = Space(id: spaceID, name: "Studio", ownerId: "self")
        let other = Space(id: "Other1234567", name: "Other", ownerId: "someone")
        model.spaces = [studio, other]
        model.detail = SpaceDetail(space: studio, channels: [], members: [])
        XCTAssertTrue(model.isOwner)

        let picture = PreparedPicture(data: Data(repeating: 1, count: 16), contentType: "image/webp", width: 512, height: 512)
        try await model.changeSpaceIcon(picture)
        XCTAssertEqual(model.detail?.space.iconPicture?.asset, pictureID)
        XCTAssertEqual(model.spaces.map { $0.iconPicture?.asset }, [pictureID, nil])
        let body = try XCTUnwrap(json(PictureURLProtocol.requests.first { $0.url.path == "/api/assets" }?.body))
        XCTAssertEqual(body["spaceId"] as? String, spaceID)
        XCTAssertEqual(body["purpose"] as? String, "spaceIcon")

        try await model.removeSpaceIcon()
        XCTAssertNil(model.detail?.space.iconPicture)
        XCTAssertEqual(model.spaces.map { $0.iconPicture?.asset }, [nil, nil])
        XCTAssertEqual(model.spaces.first?.name, "Studio")
    }

    @MainActor
    func testLiveProfileAndSpaceUpdatesReachTheRunningClient() async throws {
        PictureURLProtocol.handler = { _ in (404, Data(#"{"error":"not found"}"#.utf8)) }
        let model = AppModel(api: client())
        model.account = Account(id: "self", username: "me", displayName: "Me", avatarId: 4)
        model.phase = .ready
        let studio = Space(id: spaceID, name: "Studio", ownerId: "self")
        model.spaces = [studio, Space(id: "Other1234567", name: "Other", ownerId: "someone")]
        model.invitations = [Space(id: "Invite123456", name: "Invited", ownerId: "someone")]
        let maya = Member(id: "maya", username: "maya", displayName: "Maya", owner: false, avatarId: 31)
        model.detail = SpaceDetail(space: studio, channels: [], members: [maya])
        model.pendingMembers = [Member(id: "sam", username: "sam", displayName: "Sam", owner: false)]
        model.directMessages = [DirectMessageConversation(id: "Dm0000000001",
                                                          peer: DirectMessagePeer(id: "maya", username: "maya", displayName: "Maya"),
                                                          lastSeq: "3", readSeq: "1")]
        let author = ChatAuthor(id: "maya", name: "Maya", isGuest: false, avatarId: 31)
        model.chat.messages = [ChatMessage(id: "m1", channelId: "Design123456", seq: "1", author: author,
                                           content: ChatContent(version: 1, type: "text", text: "hi"), createdAt: "now", clientMessageId: "c1")]

        model.receiveProfileFeed(["type": "profile.updated", "person": [
            "id": "maya", "username": "maya_r", "displayName": "Maya R", "avatarId": 31, "avatarAsset": pictureID, "avatarUrl": pictureURL,
        ]])
        XCTAssertEqual(model.detail?.members.first?.displayName, "Maya R")
        XCTAssertEqual(model.detail?.members.first?.avatarPicture?.asset, pictureID)
        XCTAssertEqual(model.directMessages.first?.peer.username, "maya_r")
        XCTAssertEqual(model.directMessages.first?.unread, true, "read cursors are untouched")
        XCTAssertEqual(model.chat.messages.first?.author.name, "Maya R")
        XCTAssertEqual(model.chat.messages.first?.content.text, "hi")
        XCTAssertEqual(model.profiled(maya).avatarPicture?.asset, pictureID, "rows rendered from other lists follow too")
        XCTAssertEqual(model.pendingMembers.first?.displayName, "Sam")
        // A late read of an older list cannot bring the old name back.
        model.detail = SpaceDetail(space: studio, channels: [], members: [maya])
        XCTAssertEqual(model.detail?.members.first?.displayName, "Maya R")

        // Your own profile changed on another device.
        model.receiveProfileFeed(["type": "profile.updated", "person": [
            "id": "self", "username": "me_too", "displayName": "Me Too", "avatarAsset": pictureID, "avatarUrl": pictureURL,
        ]])
        XCTAssertEqual(model.account?.displayName, "Me Too")
        XCTAssertEqual(model.account?.avatarId, 4, "the saved mascot stays")
        XCTAssertEqual(model.account?.avatarPicture?.asset, pictureID)

        model.receiveProfileFeed(["type": "space.updated", "space": [
            "id": spaceID, "name": "Friends", "ownerId": "self", "iconAsset": pictureID, "iconUrl": pictureURL,
        ]])
        XCTAssertEqual(model.spaces.map(\.name), ["Friends", "Other"])
        XCTAssertEqual(model.spaces.first?.iconPicture?.asset, pictureID)
        XCTAssertEqual(model.detail?.space.name, "Friends")
        XCTAssertEqual(model.detail?.space.iconPicture?.asset, pictureID)
        model.receiveProfileFeed(["type": "space.updated", "space": ["id": "Unknown12345", "name": "Elsewhere", "ownerId": "x"]])
        XCTAssertEqual(model.spaces.map(\.name), ["Friends", "Other"], "spaces you don't have are ignored")
        XCTAssertEqual(model.invitations.map(\.name), ["Invited"])
        model.receiveProfileFeed(["type": "space.updated", "space": ["id": "Invite123456", "name": "Renamed invite", "ownerId": "someone"]])
        XCTAssertEqual(model.invitations.map(\.name), ["Renamed invite"])

        await model.logout()
        XCTAssertTrue(model.profileUpdates.isEmpty, "signing out forgets other people's updates")
    }

    @MainActor
    func testARefreshRepairsAMissedProfileEventForLoadedAuthors() async throws {
        PictureURLProtocol.handler = { request in
            guard request.url?.path == "/api/dms" else { return (404, Data(#"{"error":"not found"}"#.utf8)) }
            return (200, Data(#"{"conversations":[{"id":"Dm0000000001","peer":{"id":"maya","username":"maya_p2","displayName":"Maya P2","avatarId":31,"avatarAsset":"\#(pictureID)","avatarUrl":"\#(pictureURL)"},"lastSeq":"1","readSeq":"1"}]}"#.utf8))
        }
        let model = AppModel(api: client())
        model.account = Account(id: "self", username: "me", displayName: "Me")
        model.phase = .ready
        model.chat.messages = [ChatMessage(id: "m1", channelId: "Design123456", seq: "1", author: ChatAuthor(id: "maya", name: "Maya", isGuest: false),
                                           content: ChatContent(version: 1, type: "text", text: "hi"), createdAt: "now", clientMessageId: "c1")]
        // P1 arrives live.
        model.receiveProfileFeed(["type": "profile.updated", "person": ["id": "maya", "username": "maya_p1", "displayName": "Maya P1", "avatarId": 31]])
        XCTAssertEqual(model.chat.messages.first?.author.name, "Maya P1")
        // P2 is missed while disconnected; the 15-second DM refresh carries it.
        await model.refreshDirectMessages()
        XCTAssertEqual(model.chat.messages.first?.author.name, "Maya P2")
        XCTAssertEqual(model.chat.messages.first?.author.avatarPicture?.asset, pictureID)
        XCTAssertEqual(model.chat.messages.first?.content.text, "hi")
    }

    // MARK: Fixtures

    /// An opaque sRGB image from RGB values.
    private func image(width: Int, height: Int, pixel: (Int, Int) -> [UInt8]) -> CGImage {
        var bytes: [UInt8] = []
        bytes.reserveCapacity(width * height * 4)
        for y in 0..<height { for x in 0..<width { bytes += pixel(x, y) + [255] } }
        let provider = CGDataProvider(data: Data(bytes) as CFData)!
        return CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
                       space: CGColorSpace(name: CGColorSpace.sRGB)!,
                       bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue),
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    }

    private func encoded(_ image: CGImage, type: UTType, properties: [CFString: Any] = [:]) throws -> Data {
        let output = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(output as CFMutableData, type.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, image, properties as CFDictionary)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return output as Data
    }

    /// A looping GIF whose frames alternate red and blue.
    private func animatedGIF(width: Int, height: Int, frames: Int) throws -> Data {
        let output = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(output as CFMutableData, UTType.gif.identifier as CFString, frames, nil))
        CGImageDestinationSetProperties(destination, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFLoopCount: 0]] as CFDictionary)
        for index in 0..<frames {
            let frame = image(width: width, height: height) { _, _ in index % 2 == 0 ? [255, 0, 0] : [0, 0, 255] }
            CGImageDestinationAddImage(destination, frame, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 0.1]] as CFDictionary)
        }
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return output as Data
    }

    private func properties(_ data: Data) throws -> [CFString: Any] {
        let source = try XCTUnwrap(CGImageSourceCreateWithData(data as CFData, nil))
        return try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
    }
}
