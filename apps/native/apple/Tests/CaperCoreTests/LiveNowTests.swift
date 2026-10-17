import XCTest
@testable import CaperCore

/// Design H's "Live now" rules, which the call card, room cards and phone strip share.
final class LiveNowTests: XCTestCase {
    func testRoomNamesReadAsOneLine() {
        XCTAssertEqual(LiveNow.names([]), "")
        XCTAssertEqual(LiveNow.names(["Maya"]), "Maya")
        XCTAssertEqual(LiveNow.names(["Maya", "Alex"]), "Maya and Alex")
        XCTAssertEqual(LiveNow.names(["Maya", "Alex", "Sam"]), "Maya, Alex and Sam")
        XCTAssertEqual(LiveNow.names(["Maya", "Alex", "Sam", "Jo"]), "Maya, Alex and 2 more", "More than three: two names and the rest")
        XCTAssertEqual(LiveNow.names(["Maya", "Alex", "Sam", "Jo", "Ri"]), "Maya, Alex and 3 more")
    }

    func testPeopleCount() {
        XCTAssertEqual(LiveNow.people(1), "1 person")
        XCTAssertEqual(LiveNow.people(2), "2 people")
        XCTAssertEqual(LiveNow.people(12), "12 people")
    }

    func testFacesShowEveryoneUpToTenThenNineAndATile() {
        let slots = LiveNow.faceSlots(columns: 5)
        XCTAssertEqual(slots, 10, "Two rows of five")
        let ten = Array(1...10)
        XCTAssertEqual(LiveNow.fold(ten, slots: slots).shown, ten, "Up to ten people: everyone")
        XCTAssertEqual(LiveNow.fold(ten, slots: slots).folded, [])
        let eleven = Array(1...11)
        XCTAssertEqual(LiveNow.fold(eleven, slots: slots).shown, Array(1...9), "More than ten: the first nine")
        XCTAssertEqual(LiveNow.fold(eleven, slots: slots).folded, [10, 11], "The +N tile holds the rest (total − 9)")
        XCTAssertEqual(LiveNow.fold(Array(1...14), slots: slots).folded.count, 5)
    }

    func testFacesNeverExceedTwoRowsOrTen() {
        XCTAssertEqual(LiveNow.faceSlots(columns: 4), 8, "A narrow card keeps two rows")
        XCTAssertEqual(LiveNow.faceSlots(columns: 7), 10, "A wide card still shows at most ten")
        XCTAssertEqual(LiveNow.faceSlots(columns: 0), 2, "At least one face per row")
        let split = LiveNow.fold(Array(1...9), slots: 8)
        XCTAssertEqual(split.shown, Array(1...7), "The tile takes the last slot of the second row")
        XCTAssertEqual(split.folded, [8, 9])
        XCTAssertEqual(LiveNow.columns(width: 232, pitch: 44), 5)
        XCTAssertEqual(LiveNow.columns(width: 30, pitch: 44), 1)
        XCTAssertEqual(LiveNow.columns(width: .infinity, pitch: 44), 1)
    }

    func testYouComeFirstThenJoinOrder() {
        let joined = ["maya", "me", "alex", "sam"]
        XCTAssertEqual(LiveNow.selfFirst(joined) { $0 == "me" }, ["me", "maya", "alex", "sam"])
        XCTAssertEqual(LiveNow.selfFirst(["maya", "alex"]) { $0 == "me" }, ["maya", "alex"], "Not yet in the roster: join order")
    }

    func testOtherRoomsBusiestFirstThenByName() {
        let rooms = [("random", 2), ("planning", 2), ("design", 5), ("Lobby", 1), ("art", 2)]
        let ordered = LiveNow.busiestFirst(rooms, count: { $0.1 }, name: { $0.0 }).map(\.0)
        XCTAssertEqual(ordered, ["design", "art", "planning", "random", "Lobby"])
        let ties = [("general", 1, "a"), ("general", 1, "b")]
        XCTAssertEqual(LiveNow.busiestFirst(ties, count: { $0.1 }, name: { $0.0 }).map(\.2), ["a", "b"], "Equal rooms keep their order")
    }
}
