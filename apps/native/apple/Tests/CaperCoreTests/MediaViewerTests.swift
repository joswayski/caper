import CoreGraphics
import XCTest
@testable import CaperCore

final class MediaViewerTests: XCTestCase {
    private func file(_ id: String, _ kind: AttachmentKind, status: AttachmentStatus = .ready, animated: Bool = false,
                      url: String? = "https://cdn.caper.chat/original/A?exp=4102444800&sig=a", unavailable: Bool = false) -> ChatAttachment {
        ChatAttachment(id: id, kind: kind, contentType: kind == .video ? "video/mp4" : "image/webp", name: "\(id).bin", size: 1,
                       status: status, animated: animated, url: url, unavailable: unavailable)
    }

    private func message(_ id: String, _ attachments: [ChatAttachment]) -> ChatMessage {
        ChatMessage(id: id, channelId: "Chan12345678", seq: "1", author: ChatAuthor(id: "u", name: "U", isGuest: false),
                    content: ChatContent(version: 1, type: "text", text: "", attachments: attachments), createdAt: "now", clientMessageId: id)
    }

    // MARK: The set

    func testSetIsTheMessagesReadyImagesAndVideosInMessageOrder() {
        let attachments = [
            file("A", .image), file("B", .file), file("C", .video), file("D", .audio),
            file("E", .image, status: .processing, url: nil), file("F", .video, status: .failed),
            file("G", .image, unavailable: true), file("H", .video, animated: true), file("I", .image, url: nil),
        ]
        XCTAssertEqual(MediaViewerPolicy.items(attachments).map(\.id), ["A", "C", "H"], "animations are videos and open too")
        XCTAssertFalse(MediaViewerPolicy.isViewable(file("P", .image, url: "file:///tmp/caper-attachments/x/shot.png")),
                       "the pending row's local copies keep opening on their own")
        XCTAssertTrue(MediaViewerPolicy.items([]).isEmpty)
    }

    func testCounterAndStepping() {
        XCTAssertNil(MediaViewerPolicy.counter(position: 0, count: 1), "one file shows no counter")
        XCTAssertEqual(MediaViewerPolicy.counter(position: 1, count: 5), "2 / 5")
        XCTAssertEqual(MediaViewerPolicy.step(0, by: -1, count: 3), 0, "Previous stops at the first file")
        XCTAssertEqual(MediaViewerPolicy.step(0, by: 1, count: 3), 1)
        XCTAssertEqual(MediaViewerPolicy.step(2, by: 1, count: 3), 2, "Next stops at the last file")
        XCTAssertEqual(MediaViewerPolicy.step(0, by: 1, count: 0), 0)
    }

    func testLatestCopiesFollowRefreshesAndRemovalsInTheOpenedOrder() {
        let opened = [file("A", .image), file("C", .video)]
        var refreshed = file("A", .image, url: "https://cdn.caper.chat/original/A?exp=4102444900&sig=new")
        refreshed.previewUrl = "https://cdn.caper.chat/preview/A?exp=4102444900&sig=new"
        let removed = file("C", .video, url: nil, unavailable: true)
        let messages = [message("m0", [file("Z", .image)]), message("m1", [removed, file("B", .file), refreshed])]
        let latest = MediaViewerPolicy.latest(opened, in: messages)
        XCTAssertEqual(latest.map(\.id), ["A", "C"], "the set keeps its order and size")
        XCTAssertEqual(latest[0].url, refreshed.url)
        XCTAssertTrue(latest[1].unavailable)
        XCTAssertFalse(MediaViewerPolicy.isViewable(latest[1]), "a removed file shows File removed")
        XCTAssertEqual(MediaViewerPolicy.latest(opened, in: []), opened, "files no longer loaded keep the opened copy")
        let pinned = message("m1", [file("A", .image, url: "https://cdn.caper.chat/original/A?exp=1&sig=pinned")])
        XCTAssertEqual(MediaViewerPolicy.latest(opened, in: messages + [pinned])[0].url, refreshed.url, "the first loaded copy wins")
    }

    // MARK: Zoom

    func testFittingKeepsTheAspectRatio() {
        let wide = MediaViewerPolicy.fittedSize(width: 2000, height: 1000, in: CGSize(width: 400, height: 800))
        XCTAssertEqual(wide.width, 400, accuracy: 0.001)
        XCTAssertEqual(wide.height, 200, accuracy: 0.001)
        let small = MediaViewerPolicy.fittedSize(width: 100, height: 200, in: CGSize(width: 400, height: 800))
        XCTAssertEqual(small.width, 400, accuracy: 0.001, "fits the viewport")
        XCTAssertEqual(small.height, 800, accuracy: 0.001)
        XCTAssertEqual(MediaViewerPolicy.fittedSize(width: 0, height: 10, in: CGSize(width: 400, height: 800)), .zero)
    }

    func testDoubleTapCentresThePointWithinTheEdges() {
        let fitted = CGSize(width: 400, height: 200)
        let container = CGSize(width: 400, height: 800)
        // A tap right of centre moves the image left by twice the distance.
        let offset = MediaViewerPolicy.zoomOffset(at: CGPoint(x: 300, y: 100), fitted: fitted, scale: 2)
        XCTAssertEqual(offset.width, -200, accuracy: 0.001)
        XCTAssertEqual(offset.height, 0, accuracy: 0.001)
        // 800 wide at 2×: at most 200 either way. 400 tall still fits 800: no vertical pan.
        let clamped = MediaViewerPolicy.clampedOffset(CGSize(width: -350, height: 90), scale: 2, fitted: fitted, container: container)
        XCTAssertEqual(clamped.width, -200, accuracy: 0.001)
        XCTAssertEqual(clamped.height, 0, accuracy: 0.001)
        XCTAssertEqual(MediaViewerPolicy.clampedOffset(CGSize(width: 50, height: 50), scale: 1, fitted: fitted, container: container), .zero)
        XCTAssertGreaterThan(MediaViewerPolicy.maxZoom, MediaViewerPolicy.doubleTapZoom)
    }

    // MARK: Animated images

    func testFrameTimingLoopsLikeBrowsers() {
        XCTAssertEqual(MediaViewerPolicy.frameDuration(nil), 0.1)
        XCTAssertEqual(MediaViewerPolicy.frameDuration(0), 0.1)
        XCTAssertEqual(MediaViewerPolicy.frameDuration(0.01), 0.1, "10 ms or less shows for 100 ms")
        XCTAssertEqual(MediaViewerPolicy.frameDuration(0.04), 0.04)
        let durations = [0.1, 0.2, 0.3]
        XCTAssertEqual(MediaViewerPolicy.frameIndex(at: 0, durations: durations), 0)
        XCTAssertEqual(MediaViewerPolicy.frameIndex(at: 0.15, durations: durations), 1)
        XCTAssertEqual(MediaViewerPolicy.frameIndex(at: 0.35, durations: durations), 2)
        XCTAssertEqual(MediaViewerPolicy.frameIndex(at: 0.65, durations: durations), 0, "loops after the last frame")
        XCTAssertEqual(MediaViewerPolicy.frameIndex(at: 5, durations: [0.1]), 0, "a still has one frame")
        XCTAssertEqual(MediaViewerPolicy.frameIndex(at: .infinity, durations: durations), 0)
    }
}
