import XCTest
@testable import CaperCore

final class InviteLinksTests: XCTestCase {
    /// Generated from web (shared/invites/invite-cases.json): only the code
    /// comes back, never the link's host, so joins use the configured API.
    func testCodeFollowsTheSharedCases() throws {
        struct Case: Decodable { let input: String; let code: String? }
        struct Cases: Decodable { let cases: [Case] }
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        // JSONDecoder, not JSONSerialization: Darwin's JSONSerialization drops a leading U+FEFF.
        let cases = try JSONDecoder().decode(Cases.self, from: Data(contentsOf: root.appendingPathComponent("shared/invites/invite-cases.json")))
        XCTAssertGreaterThan(cases.cases.count, 50)
        for item in cases.cases { XCTAssertEqual(InviteLinks.code(from: item.input), item.code, item.input.debugDescription) }
    }

    func testOpenedLinksAreTheAppSchemeOrCaperChatUniversalLinks() throws {
        let code = "aB3dE5gH9k"
        for link in [
            "caper://invite/\(code)",
            "caper://invite/\(code)/",
            "https://caper.chat/invite/\(code)",
            "https://caper.chat/invite/\(code)/?utm=chat",
            "https://CAPER.chat/invite/\(code)",
        ] {
            XCTAssertEqual(InviteLinks.code(opening: try XCTUnwrap(URL(string: link), link)), code, link)
        }
        for link in [
            "http://caper.chat/invite/\(code)",                // universal links are HTTPS only
            "https://staging.example.test/invite/\(code)",
            "https://www.caper.chat/invite/\(code)",
            "https://caper.chat/invites/\(code)",
            "https://caper.chat/invite/aB3dE5gH9",
            "caper://invite/aB3dE5gH9",
            "caper://settings",
            "mailto:someone@caper.chat",
            "/invite/\(code)",
        ] {
            XCTAssertNil(InviteLinks.code(opening: try XCTUnwrap(URL(string: link), link)), link)
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
                       "#feedback · 7 of 10 joins remaining · Expires in 6 days")
        XCTAssertEqual(InviteLinks.meta(for: try link(channel: nil, uses: 3, maxUses: 10_000, expiresIn: 6 * 3_600), now: now),
                       "9,997 of 10,000 joins remaining · Expires in 6 hours")
        XCTAssertEqual(InviteLinks.grouped(1_234_567), "1,234,567")
        XCTAssertEqual(InviteLinks.meta(for: try link(channel: nil, uses: 2, maxUses: nil, expiresIn: nil), now: now),
                       "2 uses · Never expires")
        XCTAssertEqual(InviteLinks.meta(for: try link(channel: nil, uses: 1, maxUses: 1, expiresIn: 1_799), now: now),
                       "0 of 1 join remaining · Expires in 30 minutes")
        XCTAssertEqual(InviteLinks.meta(for: try link(channel: nil, uses: 1, maxUses: nil, expiresIn: 3_600), now: now),
                       "1 use · Expires in 1 hour")
        XCTAssertEqual(InviteLinks.expiry(nil, now: now), "Never expires")
        XCTAssertEqual(InviteLinks.memberCount(1), "1 member")
        XCTAssertEqual(InviteLinks.memberCount(12), "12 members")
    }

    func testExpiryIsAWholeNumberOfUnitsFromOneMinuteTo365Days() {
        func seconds(_ number: String, _ unit: InviteExpiryUnit) -> Result<Int?, InviteOptionError> {
            InviteLinks.expirySeconds(number, unit: unit)
        }
        XCTAssertEqual(InviteLinks.defaultExpiryNumber, "30")
        XCTAssertEqual(seconds(InviteLinks.defaultExpiryNumber, InviteLinks.defaultExpiryUnit), .success(2_592_000))
        XCTAssertEqual(InviteLinks.maxUses(InviteLinks.defaultMaxUses), .success(100))
        XCTAssertEqual(InviteLinks.defaultExpiryUnit, .days)
        XCTAssertEqual(InviteExpiryUnit.allCases.map(\.label), ["minutes", "hours", "days", "Never"])
        XCTAssertEqual(seconds("7", .days), .success(604_800))
        XCTAssertEqual(seconds("1", .minutes), .success(60))
        XCTAssertEqual(seconds("90", .minutes), .success(5_400))
        XCTAssertEqual(seconds("12", .hours), .success(43_200))
        XCTAssertEqual(seconds("8760", .hours), .success(31_536_000))
        XCTAssertEqual(seconds("525600", .minutes), .success(31_536_000))
        XCTAssertEqual(seconds("365", .days), .success(31_536_000))
        XCTAssertEqual(seconds(" 30 ", .days), .success(2_592_000))
        XCTAssertEqual(seconds("007", .hours), .success(25_200))
        // Never ignores the number, even an invalid one.
        XCTAssertEqual(seconds("7", .never), .success(nil))
        XCTAssertEqual(seconds("", .never), .success(nil))
        XCTAssertEqual(seconds("abc", .never), .success(nil))
        for (number, unit) in [
            ("", InviteExpiryUnit.days), ("  ", .days), ("0", .minutes), ("0", .days), ("-1", .days), ("+1", .days),
            ("1.5", .days), ("1,000", .minutes), ("1e2", .minutes), ("seven", .days), ("٣", .days), ("７", .days),
            ("366", .days), ("8761", .hours), ("525601", .minutes),
            ("9223372036854775807", .days), ("99999999999999999999", .minutes),
        ] {
            XCTAssertEqual(InviteLinks.expirySeconds(number, unit: unit),
                           .failure(InviteOptionError(message: "Expiry must be between 1 minute and 365 days.")), "\(number) \(unit)")
        }
    }

    func testMaxUsesIsEmptyForNoLimitOrAWholeNumberUpTo10000() {
        XCTAssertEqual(InviteLinks.maxUses(""), .success(nil))
        XCTAssertEqual(InviteLinks.maxUses("   "), .success(nil))
        XCTAssertEqual(InviteLinks.maxUses("1"), .success(1))
        XCTAssertEqual(InviteLinks.maxUses(" 25\n"), .success(25))
        XCTAssertEqual(InviteLinks.maxUses("10000"), .success(10_000))
        for text in ["0", "-1", "+5", "1.0", "2.5", "10,000", "10001", "1e3", "ten", "٥", "99999999999999999999"] {
            XCTAssertEqual(InviteLinks.maxUses(text),
                           .failure(InviteOptionError(message: "Max uses must be a whole number from 1 to 10,000.")), text)
        }
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
