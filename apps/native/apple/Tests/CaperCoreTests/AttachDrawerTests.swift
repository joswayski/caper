import XCTest
@testable import CaperCore

final class AttachDrawerTests: XCTestCase {
    private func file(_ name: String) -> LocalAttachmentFile {
        LocalAttachmentFile(url: URL(fileURLWithPath: "/tmp/\(name)"), name: name, contentType: "image/jpeg", size: 1, source: name)
    }

    func testCapacityShowsOnlyAtTheLimitAsOnWeb() {
        XCTAssertNil(AttachCapacity.label(0))
        XCTAssertNil(AttachCapacity.label(9))
        XCTAssertEqual(AttachCapacity.label(10), "10 of 10 attached")
        XCTAssertEqual(AttachCapacity.label(12), "10 of 10 attached")
        XCTAssertNil(AttachCapacity.hint(9))
        XCTAssertEqual(AttachCapacity.hint(10), "Remove a file to add another.")
    }

    func testCopiesJoinTheDraftInTapOrder() {
        var queue = AttachDrawerQueue()
        queue.enqueue("a"); queue.enqueue("b"); queue.enqueue("c")
        // "b" finishes first but waits for "a".
        XCTAssertEqual(queue.finish("b", file("b")), [])
        XCTAssertEqual(queue.finish("a", file("a")).map(\.name), ["a", "b"])
        XCTAssertEqual(queue.order, ["c"])
        XCTAssertEqual(queue.finish("c", file("c")).map(\.name), ["c"])
        XCTAssertTrue(queue.order.isEmpty)
    }

    func testFailedOrCancelledCopiesStopBlockingLaterOnes() {
        var queue = AttachDrawerQueue()
        queue.enqueue("a"); queue.enqueue("b"); queue.enqueue("c")
        XCTAssertEqual(queue.finish("c", file("c")), [])
        XCTAssertEqual(queue.finish("b", file("b")), [])
        // Cancelling "b" after it finished hands its copy back for removal.
        XCTAssertEqual(queue.cancel("b")?.name, "b")
        // "a" failing releases "c".
        XCTAssertEqual(queue.finish("a", nil).map(\.name), ["c"])
        XCTAssertFalse(queue.isPending("a"))
        // A copy finishing after its tap was undone is not added.
        XCTAssertEqual(queue.finish("a", file("a")), [])
    }

    func testAtMostSixGalleryGIFsPlayAndPlayingOnesKeepTheirSlot() {
        var gallery = AttachGalleryAnimation()
        for id in ["a", "b", "c", "d", "e", "f", "g"] { gallery.show(id) }
        XCTAssertEqual(gallery.playing, ["a", "b", "c", "d", "e", "f"])
        // A GIF scrolling away frees its slot for the next visible one.
        gallery.hide("b")
        XCTAssertEqual(gallery.playing, ["a", "c", "d", "e", "f", "g"])
        // A newcomer waits rather than displacing one already playing.
        gallery.show("h")
        XCTAssertFalse(gallery.playing.contains("h"))
        gallery.hide("a"); gallery.hide("a")
        XCTAssertEqual(gallery.playing, ["c", "d", "e", "f", "g", "h"])
    }
}
