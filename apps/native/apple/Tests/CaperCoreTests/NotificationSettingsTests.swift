import XCTest
@testable import CaperCore

/// The spec's sample settings: a space level, a timed channel mute and an off, muted DM.
private let sampleSettings = #"""
{"level":"all","mobile":"whenInactive","overrides":[
  {"spaceId":"space0000001","level":"mentions","mutedUntil":null},
  {"spaceId":"space0000001","channelId":"chan00000002","level":null,"mutedUntil":"2026-10-08T05:00:00Z"},
  {"conversationId":"dm0000000001","level":"nothing","mutedUntil":"forever"}
]}
"""#

final class NotificationSettingsTests: XCTestCase {
    private let space = NotificationScope.space("space0000001")
    private let general = NotificationScope.channel(spaceID: "space0000001", channelID: "chan00000001")
    private let design = NotificationScope.channel(spaceID: "space0000001", channelID: "chan00000002")
    private let direct = NotificationScope.direct("dm0000000001")
    private let beforeMute = Date(timeIntervalSince1970: 1_791_432_000) // 2026-10-08T04:00:00Z
    private let afterMute = Date(timeIntervalSince1970: 1_791_439_200) // 2026-10-08T06:00:00Z

    private func decode(_ json: String) throws -> NotificationSettings {
        try JSONDecoder().decode(NotificationSettings.self, from: Data(json.utf8))
    }

    private func object(_ value: some Encodable) throws -> NSDictionary {
        try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as? NSDictionary)
    }

    // MARK: Decoding

    func testSettingsDecodeEveryScope() throws {
        let settings = try decode(sampleSettings)
        XCTAssertEqual(settings.level, .all)
        XCTAssertEqual(settings.mobile, .whenInactive)
        XCTAssertEqual(settings.overrides.map(\.scope), [space, design, direct])
        XCTAssertEqual(settings.overrideLevel(for: space), .mentions)
        XCTAssertNil(settings.overrideLevel(for: design))
        XCTAssertEqual(settings.override(for: design)?.mutedUntil, .date(Date(timeIntervalSince1970: 1_791_435_600)))
        XCTAssertEqual(settings.override(for: direct)?.mutedUntil, .forever)
        XCTAssertNil(settings.override(for: general))
    }

    func testSettingsTolerateUnknownValuesAndMalformedOverrides() throws {
        let settings = try decode(#"""
        {"level":"someday","mobile":"sometimes","overrides":[
          {"level":"all"},
          {"spaceId":"space0000002","level":"later","mutedUntil":"2026-10-08T05:00:00.250Z"},
          {"spaceId":"space0000003","level":null,"mutedUntil":"tomorrow"}
        ]}
        """#)
        XCTAssertEqual(settings.level, .mentions, "an unknown level reads as mentions")
        XCTAssertEqual(settings.mobile, .whenInactive)
        XCTAssertEqual(settings.overrides.map(\.scope), [.space("space0000002"), .space("space0000003")], "an override without a scope is skipped")
        XCTAssertEqual(settings.overrides[0].level, .mentions)
        guard case let .date(fractional)? = settings.overrides[0].mutedUntil else { return XCTFail("fractional seconds parse") }
        XCTAssertEqual(fractional.timeIntervalSince1970, 1_791_435_600.25, accuracy: 0.001)
        XCTAssertNil(settings.overrides[1].mutedUntil, "an unreadable mute is ignored")
        let empty = try decode("{}")
        XCTAssertEqual(empty, NotificationSettings(), "missing fields are the defaults")
    }

    // MARK: Request bodies

    func testOverrideChangesSendOnlyTheirFieldAndNullResets() throws {
        XCTAssertEqual(try object(NotificationOverrideChange.level(.mentions)), ["level": "mentions"])
        XCTAssertEqual(try object(NotificationOverrideChange.level(nil)), ["level": NSNull()], "null resets to Default")
        XCTAssertEqual(try object(NotificationOverrideChange.mute(.forever)), ["mutedUntil": "forever"])
        XCTAssertEqual(try object(NotificationOverrideChange.mute(nil)), ["mutedUntil": NSNull()], "null unmutes")
        let timed = NotificationOverrideChange.mute(.date(Date(timeIntervalSince1970: 1_791_435_600.75)))
        XCTAssertEqual(try object(timed), ["mutedUntil": "2026-10-08T05:00:00Z"], "whole-second UTC")
        XCTAssertEqual(try object(NotificationAccountChange(level: .nothing)), ["level": "nothing"])
        XCTAssertEqual(try object(NotificationAccountChange(mobile: .always)), ["mobile": "always"])
    }

    // MARK: Labels

    func testMenuAndSettingsCopy() {
        XCTAssertEqual(NotificationLevel.allCases.map(\.accountTitle), ["All messages", "Only @mentions and DMs", "Nothing"])
        XCTAssertEqual(NotificationLevel.allCases.map(\.overrideTitle), ["All messages", "Only @mentions", "Nothing"])
        XCTAssertEqual(NotificationLevel.defaultTitle(inheriting: .all), "Default (All messages)")
        XCTAssertEqual(NotificationLevel.defaultTitle(inheriting: .mentions), "Default (Only @mentions)")
        XCTAssertEqual(MobilePushPolicy.allCases.map(\.title), ["When I'm not active elsewhere", "Always"])
        XCTAssertEqual(MutePreset.allCases.map(\.title),
                       ["For 15 minutes", "For 1 hour", "For 8 hours", "For 24 hours", "Until I turn it back on"])
        XCTAssertEqual(NotificationLabels.muteTitle(for: space, muted: false), "Mute space")
        XCTAssertEqual(NotificationLabels.muteTitle(for: space, muted: true), "Unmute space")
        XCTAssertEqual(NotificationLabels.muteTitle(for: design, muted: false), "Mute channel")
        XCTAssertEqual(NotificationLabels.muteTitle(for: design, muted: true), "Unmute channel")
        XCTAssertEqual(NotificationLabels.muteTitle(for: direct, muted: false), "Mute conversation")
        XCTAssertEqual(NotificationLabels.muteTitle(for: direct, muted: true), "Unmute conversation")
        XCTAssertEqual(NotificationLabels.directNotificationsTitle(off: false), "Turn off notifications")
        XCTAssertEqual(NotificationLabels.directNotificationsTitle(off: true), "Turn on notifications")
        XCTAssertEqual(NotificationLabels.mutedWithSpace, "Muted with the space")
    }

    func testMutePresetsEndAfterTheirDuration() {
        let now = Date(timeIntervalSince1970: 1_791_432_000)
        let ends = MutePreset.allCases.map { $0.until(from: now) }
        XCTAssertEqual(ends, [.date(now.addingTimeInterval(900)), .date(now.addingTimeInterval(3_600)),
                              .date(now.addingTimeInterval(28_800)), .date(now.addingTimeInterval(86_400)), .forever])
    }

    func testMutedStatusUsesLocalTimeAndAddsTheDateWhenNotToday() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        let locale = Locale(identifier: "en_US")
        let now = Date(timeIntervalSince1970: 1_791_478_800) // 2026-10-08T17:00:00Z, 10:00 AM PDT
        func label(_ mute: MuteUntil) -> String {
            // Newer ICU puts a narrow no-break space before AM/PM.
            NotificationLabels.mutedStatus(mute, now: now, calendar: calendar, locale: locale)
                .replacingOccurrences(of: "\u{202F}", with: " ").replacingOccurrences(of: "\u{00A0}", with: " ")
        }
        XCTAssertEqual(label(.forever), "Muted")
        XCTAssertEqual(label(.date(Date(timeIntervalSince1970: 1_791_504_000))), "Muted until 5:00 PM") // same day
        XCTAssertEqual(label(.date(Date(timeIntervalSince1970: 1_791_590_400))), "Muted until Oct 9, 5:00 PM")
        XCTAssertEqual(label(.date(Date(timeIntervalSince1970: 1_798_938_000))), "Muted until Jan 2, 2027, 5:00 PM")
    }

    // MARK: Effective state

    func testMutesInheritFromTheSpaceAndExpire() throws {
        var settings = try decode(sampleSettings)
        XCTAssertTrue(settings.isMuted(design, now: beforeMute))
        XCTAssertFalse(settings.isMuted(design, now: afterMute), "an expired mute reads as unmuted")
        XCTAssertNil(settings.ownMute(for: design, now: afterMute))
        XCTAssertFalse(settings.isMutedWithSpace(design, now: beforeMute))
        XCTAssertEqual(settings.nextMuteExpiry(after: beforeMute), Date(timeIntervalSince1970: 1_791_435_600))
        XCTAssertNil(settings.nextMuteExpiry(after: afterMute), "forever never expires")

        settings = settings.applying(.mute(.forever), to: space)
        XCTAssertTrue(settings.isMuted(general, now: afterMute), "muting a space mutes its channels")
        XCTAssertTrue(settings.isMutedWithSpace(general, now: afterMute))
        XCTAssertNil(settings.ownMute(for: general, now: afterMute), "the channel's own mute stays separate")
        XCTAssertFalse(settings.isMutedWithSpace(space, now: afterMute))
        XCTAssertTrue(settings.isMuted(direct, now: afterMute))
    }

    func testDefaultNamesTheInheritedLevel() throws {
        let settings = try decode(sampleSettings)
        XCTAssertEqual(settings.inheritedLevel(for: space), .all, "a space inherits the account level")
        XCTAssertEqual(settings.inheritedLevel(for: design), .mentions, "a channel inherits its space's level")
        XCTAssertEqual(settings.inheritedLevel(for: .channel(spaceID: "space0000009", channelID: "chan00000009")), .all)
        XCTAssertTrue(settings.notificationsOff(direct))
        XCTAssertFalse(settings.notificationsOff(.direct("dm0000000002")))
    }

    func testChangesApplyAndEmptyOverridesDrop() throws {
        let settings = try decode(sampleSettings)
        let leveled = settings.applying(.level(.nothing), to: general)
        XCTAssertEqual(leveled.overrideLevel(for: general), .nothing)
        XCTAssertNil(leveled.override(for: general)?.mutedUntil)
        let reset = settings.applying(.mute(nil), to: design)
        XCTAssertNil(reset.override(for: design), "an override with no level and no mute is dropped")
        let unchanged = settings.applying(.level(.all), to: design)
        XCTAssertEqual(unchanged.override(for: design)?.mutedUntil, settings.override(for: design)?.mutedUntil, "a level change keeps the mute")
        let restored = reset.replacing(settings.override(for: design), for: design)
        XCTAssertEqual(restored.override(for: design), settings.override(for: design), "a failed change restores the previous override")
        let account = settings.applying(NotificationAccountChange(mobile: .always))
        XCTAssertEqual(account.mobile, .always)
        XCTAssertEqual(account.level, .all, "an account change touches only its field")
    }

    // MARK: Push payloads

    func testPushPayloadsRouteToTheirConversation() {
        let channel: [AnyHashable: Any] = ["aps": ["alert": ["title": "Maya · #design (Fixture Studio)", "body": "Hi"]],
                                           "kind": "channel.message", "messageId": "chan00000002m05",
                                           "spaceId": "space0000001", "channelId": "chan00000002"]
        let dm: [AnyHashable: Any] = ["aps": ["alert": ["title": "Maya", "body": "Hi"]], "kind": "direct.message",
                                      "messageId": "dm0000000001m01", "conversationId": "dm0000000001"]
        let future: [AnyHashable: Any] = ["kind": "reaction.added", "conversationId": "dm0000000001"]
        XCTAssertEqual(NotificationRoute(userInfo: channel), .channel(spaceID: "space0000001", channelID: "chan00000002"))
        XCTAssertEqual(NotificationRoute(userInfo: dm), .direct(conversationID: "dm0000000001"))
        XCTAssertEqual(NotificationRoute(userInfo: future), .direct(conversationID: "dm0000000001"), "unknown kinds still route")
        XCTAssertNil(NotificationRoute(userInfo: ["spaceId": "space0000001"]))
        XCTAssertNil(NotificationRoute(userInfo: ["conversationId": "", "channelId": 4]))

        let route = NotificationRoute.channel(spaceID: "space0000001", channelID: "chan00000002")
        XCTAssertTrue(route.isOpen(spaceID: "space0000001", channelID: "chan00000002", directMessageID: nil))
        XCTAssertFalse(route.isOpen(spaceID: "space0000001", channelID: "chan00000001", directMessageID: nil))
        XCTAssertFalse(route.isOpen(spaceID: "space0000001", channelID: "chan00000002", directMessageID: "dm0000000001"),
                       "a DM covers the channel that stays selected behind it")
        XCTAssertTrue(NotificationRoute.direct(conversationID: "dm0000000001")
            .isOpen(spaceID: "space0000001", channelID: nil, directMessageID: "dm0000000001"))
        XCTAssertFalse(NotificationRoute.direct(conversationID: "dm0000000001")
            .isOpen(spaceID: nil, channelID: nil, directMessageID: "dm0000000002"))
    }
}

// MARK: API and model

private final class NotificationsMockURLProtocol: URLProtocol, @unchecked Sendable {
    static var handler: ((URLRequest) throws -> (Int, Data))!
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            let (status, data) = try Self.handler(request)
            let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil,
                                           headerFields: ["content-type": "application/json"])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}

private final class NotificationsTokenStore: TokenStore, @unchecked Sendable {
    func load() throws -> String? { "account-secret" }
    func save(_ token: String) throws {}
    func clear() throws {}
}

private func requestJSON(_ request: URLRequest) throws -> NSDictionary? {
    var body = request.httpBody
    if body == nil, let stream = request.httpBodyStream {
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = stream.read(&buffer, maxLength: buffer.count)
            if count <= 0 { break }
            data.append(buffer, count: count)
        }
        body = data
    }
    guard let body, !body.isEmpty else { return nil }
    return try JSONSerialization.jsonObject(with: body) as? NSDictionary
}

@MainActor
final class NotificationModelTests: XCTestCase {
    override func tearDown() {
        NotificationsMockURLProtocol.handler = nil
        super.tearDown()
    }

    private func client() -> APIClient {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [NotificationsMockURLProtocol.self]
        return APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration),
                         tokenStore: NotificationsTokenStore())
    }

    func testNotificationRoutesAndBodies() async throws {
        let api = client()
        var requests: [(String, NSDictionary?)] = []
        NotificationsMockURLProtocol.handler = { request in
            let line = "\(request.httpMethod ?? "") \(request.url!.path)"
            requests.append((line, try requestJSON(request)))
            XCTAssertEqual(request.value(forHTTPHeaderField: "authorization"), "Bearer account-secret")
            switch line {
            case "GET /api/notifications/settings": return (200, Data(sampleSettings.utf8))
            case "PUT /api/notifications/settings": return (200, Data(#"{"level":"mentions","mobile":"whenInactive","overrides":[]}"#.utf8))
            case "PUT /api/spaces/space0000001/notifications":
                return (200, Data(#"{"spaceId":"space0000001","level":null,"mutedUntil":"forever"}"#.utf8))
            case "PUT /api/spaces/space0000001/channels/chan00000002/notifications":
                return (200, Data(#"{"spaceId":"space0000001","channelId":"chan00000002","level":"nothing","mutedUntil":null}"#.utf8))
            case "PUT /api/dms/dm0000000001/notifications":
                return (200, Data(#"{"conversationId":"dm0000000002","level":"nothing","mutedUntil":null}"#.utf8))
            case "POST /api/push/devices", "DELETE /api/push/devices": return (204, Data())
            default: throw URLError(.badURL)
            }
        }
        let loaded = try await api.notificationSettings()
        XCTAssertEqual(loaded.overrides.count, 3)
        let saved = try await api.updateNotificationSettings(NotificationAccountChange(level: .mentions))
        XCTAssertEqual(saved.level, .mentions)
        let space = try await api.updateNotificationOverride(.space("space0000001"), change: .mute(.forever))
        XCTAssertEqual(space.mutedUntil, .forever)
        let channel = try await api.updateNotificationOverride(.channel(spaceID: "space0000001", channelID: "chan00000002"),
                                                               change: .level(.nothing))
        XCTAssertEqual(channel.level, .nothing)
        do {
            _ = try await api.updateNotificationOverride(.direct("dm0000000001"), change: .level(nil))
            XCTFail("Another conversation's override must be rejected")
        } catch let error as APIError { XCTAssertEqual(error.status, 502) }
        do {
            _ = try await api.updateNotificationOverride(.direct("../account"), change: .level(nil))
            XCTFail("Expected local ID rejection")
        } catch let error as APIError { XCTAssertEqual(error.status, 400) }
        let token = String(repeating: "ab", count: 32)
        try await api.registerPushDevice(platform: "apns", token: token, appID: "chat.caper.ios")
        try await api.unregisterPushDevice(platform: "apns", token: token, appID: "chat.caper.ios")

        XCTAssertEqual(requests.map(\.0), [
            "GET /api/notifications/settings", "PUT /api/notifications/settings", "PUT /api/spaces/space0000001/notifications",
            "PUT /api/spaces/space0000001/channels/chan00000002/notifications", "PUT /api/dms/dm0000000001/notifications",
            "POST /api/push/devices", "DELETE /api/push/devices",
        ])
        XCTAssertNil(requests[0].1)
        XCTAssertEqual(requests[1].1, ["level": "mentions"])
        XCTAssertEqual(requests[2].1, ["mutedUntil": "forever"])
        XCTAssertEqual(requests[3].1, ["level": "nothing"])
        XCTAssertEqual(requests[4].1, ["level": NSNull()])
        XCTAssertEqual(requests[5].1, ["platform": "apns", "token": token, "appId": "chat.caper.ios"])
        XCTAssertEqual(requests[6].1, ["platform": "apns", "token": token, "appId": "chat.caper.ios"])
    }

    func testChangesAreOptimisticRevertOnFailureAndClearOnLogout() async throws {
        let model = AppModel(api: client())
        model.account = Account(id: "owner0000001", username: "owner", displayName: "Owner")
        var failing = false
        NotificationsMockURLProtocol.handler = { request in
            let line = "\(request.httpMethod ?? "") \(request.url!.path)"
            if failing, request.httpMethod == "PUT" { return (503, Data(#"{"error":"unavailable"}"#.utf8)) }
            switch line {
            case "GET /api/notifications/settings": return (200, Data(sampleSettings.utf8))
            case "PUT /api/notifications/settings": return (200, Data(#"{"level":"mentions","mobile":"always","overrides":[]}"#.utf8))
            case "PUT /api/spaces/space0000001/channels/chan00000001/notifications":
                return (200, Data(#"{"spaceId":"space0000001","channelId":"chan00000001","level":null,"mutedUntil":"forever"}"#.utf8))
            case "PUT /api/dms/dm0000000001/notifications":
                return (200, Data(#"{"conversationId":"dm0000000001","level":null,"mutedUntil":"forever"}"#.utf8))
            case "POST /api/auth/logout": return (204, Data())
            default: throw URLError(.badURL)
            }
        }
        let general = NotificationScope.channel(spaceID: "space0000001", channelID: "chan00000001")
        let direct = NotificationScope.direct("dm0000000001")
        XCTAssertNil(model.notificationSettings, "menus stay disabled until settings load")
        await model.setMute(.forever, for: general)
        XCTAssertNil(model.notificationSettings, "nothing is guessed before the first load")

        await model.loadNotificationSettings()
        XCTAssertEqual(model.notificationSettings?.overrides.count, 3)
        XCTAssertFalse(model.notificationsMuted(general))
        await model.setMute(.forever, for: general)
        XCTAssertEqual(model.notificationMute(general), .forever)
        XCTAssertTrue(model.notificationsMuted(general))

        failing = true
        await model.setMute(nil, for: general)
        XCTAssertEqual(model.notificationMute(general), .forever, "a failed unmute reverts")
        XCTAssertEqual(model.notificationError, NotificationLabels.saveFailed)
        await model.setNotificationLevel(nil, for: direct)
        XCTAssertTrue(model.notificationSettings?.notificationsOff(direct) == true, "a failed turn-on reverts")
        await model.setAccountNotificationLevel(.nothing)
        XCTAssertEqual(model.notificationSettings?.level, .all)

        failing = false
        await model.setAccountNotificationLevel(.mentions)
        XCTAssertNil(model.notificationError, "a new change clears the last error")
        XCTAssertEqual(model.notificationSettings?.level, .mentions)
        XCTAssertEqual(model.notificationSettings?.mobile, .whenInactive, "only the changed field is taken from the response")
        XCTAssertEqual(model.notificationSettings?.overrides.count, 4, "overrides, including the new channel mute, are kept")
        await model.setNotificationLevel(nil, for: direct)
        XCTAssertFalse(model.notificationSettings?.notificationsOff(direct) == true)
        XCTAssertEqual(model.notificationMute(direct), .forever)

        model.selectedSpaceID = "space0000001"
        model.selectedChannelID = "chan00000001"
        XCTAssertTrue(model.isShowing(.channel(spaceID: "space0000001", channelID: "chan00000001")))
        XCTAssertFalse(model.isShowing(.direct(conversationID: "dm0000000001")))

        await model.logout()
        XCTAssertNil(model.notificationSettings)
        XCTAssertNil(model.notificationError)
        XCTAssertFalse(model.notificationsMuted(general))
    }
}
