import XCTest
@testable import CaperCore

final class InviteLinksTests: XCTestCase {
    func testCodeFromFullLinksBareHostsAndCodes() {
        let code = "aB3dE5gH9k"
        for input in [
            code,
            "  \(code)\n",
            "https://caper.chat/invite/\(code)",
            "https://caper.chat/invite/\(code)/",
            "https://caper.chat/invite/\(code)?utm=chat",
            "https://caper.chat/invite/\(code)/?utm=chat#top",
            "HTTPS://caper.chat/invite/\(code)",
            "http://localhost:5173/invite/\(code)",
            "https://staging.example.test/invite/\(code)",
            "caper.chat/invite/\(code)",
            "caper.chat/invite/\(code)/",
            "caper.chat/invite/\(code)?ref=https://elsewhere.test",
        ] {
            XCTAssertEqual(InviteLinks.code(from: input), code, input)
        }
    }

    func testCodeRejectsAnythingElse() {
        for input in [
            "",
            "   ",
            "aB3dE5gH9",                            // 9 characters
            "aB3dE5gH9kZ",                          // 11 characters
            "aB3dE5gH9-",                           // not alphanumeric
            "aB3dé5gH9k",                           // not ASCII
            "https://caper.chat/invite/",
            "https://caper.chat/invite/aB3dE5gH9",
            "https://caper.chat/invites/aB3dE5gH9k",
            "https://caper.chat/Invite/aB3dE5gH9k",
            "https://caper.chat/invite/aB3dE5gH9k/extra",
            "https://caper.chat/invite/aB3dE5gH9k//",
            "https://caper.chat/x/invite/aB3dE5gH9k",
            "https://caper.chat/aB3dE5gH9k",
            "ftp://caper.chat/invite/aB3dE5gH9k",
            "https:///invite/aB3dE5gH9k",
            "/invite/aB3dE5gH9k",
            "invite/aB3dE5gH9k",
            "caper chat/invite/aB3dE5gH9k",
        ] {
            XCTAssertNil(InviteLinks.code(from: input), input)
        }
    }

    func testLinkUsesTheConfiguredOrigin() {
        XCTAssertEqual(InviteLinks.url(code: "aB3dE5gH9k", baseURL: URL(string: "https://caper.chat")!),
                       "https://caper.chat/invite/aB3dE5gH9k")
        XCTAssertEqual(InviteLinks.url(code: "aB3dE5gH9k", baseURL: URL(string: "http://localhost:5173/api/")!),
                       "http://localhost:5173/invite/aB3dE5gH9k")
    }

    func testMetaLineWording() throws {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        func link(channel: String?, uses: Int, maxUses: Int?, expiresIn seconds: TimeInterval?) throws -> InviteLink {
            var json: [String: Any] = [
                "code": "aB3dE5gH9k", "uses": uses,
                "createdBy": ["username": "owner", "displayName": "Owner", "avatarId": 1],
                "createdAt": "2027-01-01T00:00:00Z",
            ]
            if let channel { json["channel"] = ["id": "Channel12345", "name": channel] }
            if let maxUses { json["maxUses"] = maxUses }
            if let seconds { json["expiresAt"] = ISO8601DateFormatter().string(from: now.addingTimeInterval(seconds)) }
            return try JSONDecoder().decode(InviteLink.self, from: JSONSerialization.data(withJSONObject: json))
        }
        XCTAssertEqual(InviteLinks.meta(for: try link(channel: "feedback", uses: 3, maxUses: 10, expiresIn: 6 * 86_400), now: now),
                       "#feedback · 3 of 10 uses · Expires in 6 days")
        XCTAssertEqual(InviteLinks.meta(for: try link(channel: nil, uses: 2, maxUses: nil, expiresIn: nil), now: now),
                       "2 uses · Never expires")
        XCTAssertEqual(InviteLinks.meta(for: try link(channel: nil, uses: 1, maxUses: 1, expiresIn: 1_799), now: now),
                       "1 of 1 use · Expires in 30 minutes")
        XCTAssertEqual(InviteLinks.meta(for: try link(channel: nil, uses: 1, maxUses: nil, expiresIn: 3_600), now: now),
                       "1 use · Expires in 1 hour")
        XCTAssertEqual(InviteLinks.expiry(nil, now: now), "Never expires")
        XCTAssertEqual(InviteLinks.memberCount(1), "1 member")
        XCTAssertEqual(InviteLinks.memberCount(12), "12 members")
    }

    func testTimestampsWithMicrosecondsParse() {
        let date = InviteLinks.date("2026-10-18T12:00:00.123456Z")
        XCTAssertEqual(date.map { Int($0.timeIntervalSince1970) }, 1_792_324_800)
        XCTAssertNotNil(InviteLinks.date("2026-10-18T12:00:00Z"))
        XCTAssertNil(InviteLinks.date("soon"))
    }

    func testInvalidLinkErrorReadsAsASentence() {
        XCTAssertEqual(FriendlyError.message(for: APIError(status: 404, message: "invite link is invalid or has expired")),
                       "This invite link is invalid or has expired.")
        XCTAssertEqual(FriendlyError.message(for: APIError(status: 403, message: "you can't join this space with a link; ask the owner to invite you")),
                       "You can't join this space with a link. Ask the owner to invite you.")
    }
}
