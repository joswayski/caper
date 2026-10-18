import XCTest
@testable import CaperCore

/// Design H's voice rules, which the call card, room cards and phone strip share.
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

    func testStripShowsTheCardsFacesUpToFiveThenFourAndAChip() {
        let card = LiveNow.faceSlots(columns: 5)
        for count in 1...5 {
            let people = LiveNow.selfFirst(Array(1...count)) { $0 == count }
            XCTAssertEqual(LiveNow.stripFold(people).shown, LiveNow.fold(people, slots: card).shown,
                           "Up to five: the strip shows exactly the card's faces, in its order (\(count))")
            XCTAssertEqual(LiveNow.stripFold(people).folded, [], "Up to five: no chip (\(count))")
        }
        for count in 6...14 {
            let people = LiveNow.selfFirst(Array(1...count)) { $0 == count }
            let strip = LiveNow.stripFold(people)
            XCTAssertEqual(strip.shown, Array(people.prefix(4)), "More than five: the card's first four (\(count))")
            XCTAssertEqual(strip.folded.count, count - 4, "The chip holds the rest (total − 4)")
            XCTAssertEqual(strip.shown, Array(LiveNow.fold(people, slots: card).shown.prefix(4)), "Same people and order as the card")
        }
        XCTAssertEqual(LiveNow.stripFold(LiveNow.selfFirst([1, 2, 7]) { $0 == 7 }).shown, [7, 1, 2], "You lead the strip, as on the card")
    }

    func testCollapsedChannelsKeepTheirLiveRows() {
        let rows = [("general", false), ("design", true), ("planning", false), ("random", true)]
        XCTAssertEqual(LiveNow.listed(rows, expanded: true) { $0.1 }.map(\.0), ["general", "design", "planning", "random"])
        XCTAssertEqual(LiveNow.listed(rows, expanded: false) { $0.1 }.map(\.0), ["design", "random"],
                       "Collapsed: only quiet rows hide; live ones stay in list order")
        XCTAssertEqual(LiveNow.listed(rows.filter { !$0.1 }, expanded: false) { $0.1 }.count, 0)
    }

    func testACallWithoutARowSitsAboveTheChannels() {
        let rows = ["general", "design"]
        XCTAssertTrue(LiveNow.hasRow("design", rows: rows), "A call in a listed channel grows in its row")
        XCTAssertFalse(LiveNow.hasRow("lobby", rows: rows), "Another space's (or a visit's) call keeps its card above the channels")
        XCTAssertFalse(LiveNow.hasRow("general", rows: []), "A space without joined channels lists no rows")
    }
}
