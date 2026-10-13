import XCTest
@testable import CaperCore

final class PeopleSuggestionsTests: XCTestCase {
    private let selfID = "Self00000001"
    private var people: [Person] {
        [
            Person(id: "Alice0000001", username: "alice", displayName: "Alice Smith", avatarId: 3),
            Person(id: "Alicia000001", username: "alicia", displayName: "Alicia Jones"),
            Person(id: "Bob000000001", username: "bob", displayName: "Bob Alder"),
            Person(id: "Malice000001", username: "malice", displayName: "Mal Ice"),
            Person(id: "Zed000000001", username: "zed", displayName: "Alfie Zed"),
        ]
    }
    private var candidates: [MentionCandidate] { people.map { MentionCandidate($0) } }
    private func member(_ id: String, _ username: String, owner: Bool = false) -> Member {
        Member(id: id, username: username, displayName: username.capitalized, owner: owner)
    }
    private func names(_ text: String, _ candidates: [MentionCandidate]? = nil) -> [String] {
        PeopleSuggestions.match(text: text, candidates: candidates ?? self.candidates).map(\.username)
    }

    // MARK: Query and ranking

    func testQueryIsTrimmedWithOneLeadingAtStrippedAndLowercased() {
        XCTAssertEqual(PeopleSuggestions.query("  @Ali \n"), "ali")
        XCTAssertEqual(PeopleSuggestions.query("ALICE"), "alice")
        XCTAssertEqual(PeopleSuggestions.query("@@al"), "@al", "only one @ is stripped")
        for text in ["", "   ", "@", " @ "] { XCTAssertNil(PeopleSuggestions.query(text), text) }
        XCTAssertEqual(names(""), [], "an empty field shows no list")
        XCTAssertEqual(names("@"), [])
        XCTAssertEqual(names("@@al"), [])
    }

    func testRankingIsTheMentionRanking() {
        XCTAssertEqual(names("alice"), ["alice", "malice"], "exact username, then contains")
        XCTAssertEqual(names("@ALI"), ["alice", "alicia", "malice"], "a leading @ and case are ignored")
        XCTAssertEqual(names("al"), ["alice", "alicia", "bob", "zed", "malice"],
                       "prefix, then display-name words (ties by username), then contains")
        XCTAssertEqual(names("jones"), ["alicia"], "a later display-name word matches")
        XCTAssertEqual(names("ice"), ["malice", "alice"], "display-name word ranks above username contains")
        XCTAssertEqual(names("nobody"), [])
        for query in ["al", "ice", "a", "zed", "e"] {
            XCTAssertEqual(PeopleSuggestions.match(text: query, candidates: candidates),
                           // People only: the team special ("a" offers @admin) is for the composer.
                           MentionAutocomplete.rank(query: query, source: MentionSource(members: candidates, specials: false, feedback: false)),
                           "the fields and @ suggestions share one ranking: \(query)")
        }
        XCTAssertEqual(MentionAutocomplete.personRank(query: "alice", username: "Alice", displayName: "A"), 0)
        XCTAssertEqual(MentionAutocomplete.personRank(query: "al", username: "alice", displayName: "A"), 1)
        XCTAssertEqual(MentionAutocomplete.personRank(query: "jo", username: "x", displayName: "Alicia Jones"), 2)
        XCTAssertEqual(MentionAutocomplete.personRank(query: "lic", username: "alice", displayName: "A"), 3)
        XCTAssertNil(MentionAutocomplete.personRank(query: "zz", username: "alice", displayName: "Alice Smith"))
    }

    func testTiesSortByLowercaseUsernameAndCapAtSix() {
        let mixed = [
            MentionCandidate.member(id: "B", username: "xb", displayName: "B"),
            MentionCandidate.member(id: "A", username: "XA", displayName: "A"),
        ]
        XCTAssertEqual(names("x", mixed), ["XA", "xb"])
        let many = (0..<10).map { MentionCandidate.member(id: "User0000000\($0)", username: "user\($0)", displayName: "User \($0)") }
        XCTAssertEqual(names("user", many), ["user0", "user1", "user2", "user3", "user4", "user5"])
    }

    // MARK: Candidates

    func testSpaceInviteExcludesMembersByIDAndMarksPendingInvitees() {
        let known = people + [Person(id: "Sam000000001", username: "sam", displayName: "Sam Lee")]
        // Matched by id: a member renamed since the people list loaded is still excluded.
        let members = [member(selfID, "fixture_self", owner: true), member("Alice0000001", "alice_renamed")]
        let invited = [member("Bob000000001", "bob")]
        let candidates = PeopleSuggestions.spaceInvite(people: known, members: members, invited: invited, accountID: selfID)
        XCTAssertEqual(candidates.map(\.username), ["alicia", "malice", "zed", "sam", "bob"])
        XCTAssertEqual(candidates.filter(\.invited).map(\.username), ["bob"], "pending invitees are marked, not hidden")
        XCTAssertEqual(names("al", candidates), ["alicia", "zed", "malice"])
        XCTAssertEqual(PeopleSuggestions.spaceInvite(people: nil, members: [], invited: [], accountID: selfID), [],
                       "nothing to suggest until people load")
        XCTAssertEqual(PeopleSuggestions.spaceInvite(people: nil, members: [], invited: invited, accountID: selfID).map(\.invited),
                       [true], "invitees show before people load")
        let withSelf = [Person(id: selfID, username: "fixture_self", displayName: "Me")]
        XCTAssertEqual(PeopleSuggestions.spaceInvite(people: withSelf, members: [], invited: [], accountID: selfID), [],
                       "never yourself")
        XCTAssertEqual(PeopleSuggestions.spaceInvite(people: people, members: [], invited: [], accountID: nil).first?.avatarId, 3)
    }

    func testChannelGrantOffersSpaceMembersWithoutAccessAndMarksPendingInvitees() {
        let space = [
            member(selfID, "fixture_self", owner: true), member("Maya00000001", "maya"),
            member("Alex00000001", "alex"), member("Bob000000001", "bob"), member("Sam000000001", "sam"),
        ]
        let access = [member("Maya00000001", "maya")]
        let invited = [member("Bob000000001", "bob")]
        let candidates = PeopleSuggestions.channelGrant(spaceMembers: space, channelMembers: access, invited: invited, accountID: selfID)
        XCTAssertEqual(candidates.map(\.username), ["alex", "sam", "bob"], "never yourself, even without channel access")
        XCTAssertEqual(candidates.filter(\.invited).map(\.username), ["bob"], "pending invitees are marked, not hidden")
        XCTAssertEqual(names("ma", candidates), [], "someone with access is not suggested")
        XCTAssertEqual(names("a", candidates), ["alex", "sam"])
        XCTAssertEqual(PeopleSuggestions.channelGrant(spaceMembers: [], channelMembers: [], invited: [], accountID: selfID), [])
    }

    func testDirectMessageOffersKnownPeopleNeverYourself() {
        let known = people + [Person(id: selfID, username: "fixture_self", displayName: "Alice Myself")]
        let candidates = PeopleSuggestions.directMessage(people: known, accountID: selfID)
        XCTAssertEqual(candidates.map(\.username), ["alice", "alicia", "bob", "malice", "zed"])
        XCTAssertFalse(names("myself", candidates).contains("fixture_self"))
        XCTAssertEqual(PeopleSuggestions.directMessage(people: nil, accountID: selfID), [])
    }

    // MARK: Field state

    func testListShowsOnlyWhileFocusedAndStaysClosedForPickedText() {
        var state = PeopleSuggestionState()
        XCTAssertEqual(state.choices(text: "al", candidates: candidates, focused: true).count, 5)
        XCTAssertEqual(state.choices(text: "al", candidates: candidates, focused: false), [], "hidden without focus")
        // A pick fills in the username and closes the list until the text changes.
        state.close(for: "alice")
        XCTAssertEqual(state.choices(text: "alice", candidates: candidates, focused: true), [])
        state.textChanged(to: "alice")
        XCTAssertEqual(state.choices(text: "alice", candidates: candidates, focused: true), [], "the pick's own edit keeps it closed")
        state.textChanged(to: "alic")
        XCTAssertNil(state.closedFor)
        XCTAssertEqual(state.choices(text: "alic", candidates: candidates, focused: true).map(\.username), ["alice", "alicia", "malice"])
        // Escape closes it the same way.
        state.close(for: "alic")
        XCTAssertEqual(state.choices(text: "alic", candidates: candidates, focused: true), [])
        state.textChanged(to: "")
        state.textChanged(to: "alic")
        XCTAssertFalse(state.choices(text: "alic", candidates: candidates, focused: true).isEmpty, "retyping reopens it")
    }

    func testHighlightWrapsAndResetsOnEdit() {
        var state = PeopleSuggestionState()
        let choices = state.choices(text: "al", candidates: candidates, focused: true)
        XCTAssertEqual(state.highlightedChoice(in: choices)?.username, "alice", "Return picks the first row by default")
        state.move(down: false, count: choices.count)
        XCTAssertEqual(state.highlightedChoice(in: choices)?.username, "malice", "Up from the first row wraps to the last")
        state.move(down: true, count: choices.count)
        state.move(down: true, count: choices.count)
        XCTAssertEqual(state.highlightedChoice(in: choices)?.username, "alicia")
        state.highlighted = 9
        XCTAssertEqual(state.highlightedChoice(in: choices)?.username, "malice", "a stale highlight clamps to the list")
        state.textChanged(to: "ali")
        XCTAssertEqual(state.highlighted, 0)
        XCTAssertNil(state.highlightedChoice(in: []), "Return submits when no list is open")
        state.move(down: true, count: 0)
        XCTAssertEqual(state.highlighted, 0)
    }
}
