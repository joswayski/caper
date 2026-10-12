import XCTest
@testable import CaperCore

private final class FeedbackTokenStore: TokenStore, @unchecked Sendable {
    var token: String? = "account-secret"
    func load() throws -> String? { token }
    func save(_ token: String) throws { self.token = token }
    func clear() throws { token = nil }
}

private final class FeedbackMockURLProtocol: URLProtocol, @unchecked Sendable {
    static var handler: ((URLRequest) throws -> (Int, Data))!
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            let (status, data) = try Self.handler(request)
            client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil,
                                                                  headerFields: ["content-type": "application/json"])!,
                                cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}

private func feedbackBody(_ request: URLRequest) throws -> Data {
    if let body = request.httpBody { return body }
    guard let stream = request.httpBodyStream else { return Data() }
    stream.open()
    defer { stream.close() }
    var body = Data()
    var buffer = [UInt8](repeating: 0, count: 4096)
    while true {
        let count = stream.read(&buffer, maxLength: buffer.count)
        if count <= 0 { break }
        body.append(buffer, count: count)
    }
    return body
}

final class FeedbackTests: XCTestCase {
    private let selfID = "Team00000001"
    private let channelID = "Chan00000001"
    private let rootID = "Root00000000001"
    private let replyID = "Repl00000000001"

    private func client() -> APIClient {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [FeedbackMockURLProtocol.self]
        return APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration),
                         tokenStore: FeedbackTokenStore())
    }

    private let samplePage = #"""
    {"reports":[
      {"id":"Repl00000000001","messageId":"Repl00000000001","threadRootId":"Root00000000001","createdAt":"2026-10-11T19:00:00.123456+00:00",
       "text":"@feedback uploads fail\nfrom the share sheet","attachments":2,
       "reporter":{"id":"Alice0000001","username":"alice","displayName":"Alice","avatarId":12},
       "channel":{"id":"Chan00000001","name":"plans","direct":false},"space":{"id":"Space0000001","name":"Studio"},
       "replyCount":3,"answered":true,"archived":false},
      {"id":"Note00000000001","messageId":"Note00000000001","threadRootId":"Note00000000001","createdAt":"2026-10-11T18:00:00Z",
       "text":"","attachments":1,
       "reporter":{"id":"Bob000000001","username":"bob","displayName":"Bob","avatarId":null},
       "channel":{"id":"Dm0000000001","name":"Alice & Bob","direct":true},"space":null,
       "replyCount":0,"answered":false},
      {"id":"Broken00000001","channel":{"id":"x"}}
    ],"cursor":"123"}
    """#

    // MARK: Decoding

    func testAccountFeedbackTeamFlagIsOptional() throws {
        let team = try JSONDecoder().decode(Account.self, from: Data(#"{"id":"a","username":"a","displayName":"A","feedbackTeam":true}"#.utf8))
        XCTAssertTrue(team.isFeedbackTeam)
        let member = try JSONDecoder().decode(Account.self, from: Data(#"{"id":"b","username":"b","displayName":"B"}"#.utf8))
        XCTAssertNil(member.feedbackTeam, "absent for everyone else")
        XCTAssertFalse(member.isFeedbackTeam)
        XCTAssertFalse(Account(id: "c", feedbackTeam: false).isFeedbackTeam)
    }

    func testReportsPageDecodesAndSkipsMalformedReports() throws {
        let page = try JSONDecoder().decode(FeedbackReportsPage.self, from: Data(samplePage.utf8))
        XCTAssertEqual(page.cursor, "123")
        XCTAssertEqual(page.reports.map(\.id), [replyID, "Note00000000001"], "a malformed report is skipped, not fatal")
        let first = page.reports[0]
        XCTAssertEqual(first.threadRootId, rootID)
        XCTAssertEqual(first.attachments, 2)
        XCTAssertEqual(first.reporter, FeedbackReport.Reporter(id: "Alice0000001", username: "alice", displayName: "Alice", avatarId: 12))
        XCTAssertEqual(first.channel, FeedbackReport.Conversation(id: channelID, name: "plans", direct: false))
        XCTAssertEqual(first.space, FeedbackReport.SpaceReference(id: "Space0000001", name: "Studio"))
        XCTAssertEqual(first.replyCount, 3)
        XCTAssertTrue(first.answered)
        XCTAssertFalse(first.archived)
        let direct = page.reports[1]
        XCTAssertNil(direct.space)
        XCTAssertNil(direct.reporter.avatarId)
        XCTAssertFalse(direct.answered)

        let end = try JSONDecoder().decode(FeedbackReportsPage.self, from: Data(#"{"reports":[],"cursor":null}"#.utf8))
        XCTAssertNil(end.cursor)
        XCTAssertEqual(end.reports, [])
        let sparse = try JSONDecoder().decode(FeedbackReport.self, from: Data(#"""
        {"messageId":"Msg000000000001","reporter":{"id":"r","username":"r","displayName":"R"},"channel":{"id":"c","name":"general","direct":false},"space":{"id":"","name":""}}
        """#.utf8))
        XCTAssertEqual(sparse.id, "Msg000000000001", "the id defaults to the message")
        XCTAssertEqual(sparse.threadRootId, "Msg000000000001", "a top-level report is its own thread")
        XCTAssertEqual(sparse.text, "")
        XCTAssertNil(sparse.space, "an empty space is a DM")
        XCTAssertFalse(sparse.archived, "missing means not archived")
        let archived = try JSONDecoder().decode(FeedbackReport.self, from: Data(#"""
        {"messageId":"Msg000000000002","reporter":{"id":"r","username":"r","displayName":"R"},"channel":{"id":"c","name":"general","direct":false},"archived":true}
        """#.utf8))
        XCTAssertTrue(archived.archived)
    }

    func testVisitorAdminAndFeedbackMentionDecode() throws {
        let history = try JSONDecoder().decode(ChatHistory.self, from: Data(#"""
        {"space":{"id":"","name":"Direct messages"},"channel":{"id":"Dm0000000001","name":"Alice & Bob","direct":true},
         "messages":[{"id":"Msg000000000001","channelId":"Dm0000000001","seq":"4",
           "author":{"id":"Team00000001","name":"Jose","isGuest":false,"admin":true},
           "content":{"version":1,"type":"text","text":"@Caper help","mentions":[{"type":"feedback"},{"type":"later"}]},
           "createdAt":"now","clientMessageId":"c"}],
         "cursor":"4","hasMore":false,"visitor":true}
        """#.utf8))
        XCTAssertTrue(history.visitor)
        XCTAssertEqual(history.channel?.name, "Alice & Bob")
        XCTAssertTrue(history.messages[0].author.isAdmin)
        XCTAssertEqual(history.messages[0].content.mentions, [.feedback], "unknown mention types are still skipped")
        let member = try JSONDecoder().decode(ChatHistory.self, from: Data(#"{"messages":[],"cursor":"0","hasMore":false}"#.utf8))
        XCTAssertFalse(member.visitor, "missing means a member")
        XCTAssertFalse(ChatAuthor(id: "x", name: "X", isGuest: false).isAdmin)
        XCTAssertFalse(try JSONDecoder().decode(ChatAuthor.self, from: Data(#"{"id":"x","name":"X","isGuest":false,"admin":false}"#.utf8)).isAdmin)

        let thread = try JSONDecoder().decode(ThreadHistory.self, from: Data(#"""
        {"root":{"id":"Root00000000001","channelId":"Chan00000001","seq":"1","author":{"id":"a","name":"A","isGuest":false},
          "content":{"version":1,"type":"text","text":"root"},"createdAt":"now","clientMessageId":"r"},
         "messages":[],"cursor":"1","hasMore":false,"hasNewer":false,"visitor":true}
        """#.utf8))
        XCTAssertEqual(thread.visitor, true)

        let content = ChatContent(version: 1, type: "text", text: "@feedback", mentions: [.feedback])
        XCTAssertEqual(try JSONDecoder().decode(ChatContent.self, from: JSONEncoder().encode(content)), content)
        let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(content)) as? [String: Any]
        XCTAssertEqual((encoded?["mentions"] as? [[String: String]])?.first, ["type": "feedback"])
    }

    // MARK: Messages

    private func message(_ text: String, _ mentions: [MessageMention], author: String = "Other0000001") -> ChatMessage {
        ChatMessage(id: "m", channelId: channelID, seq: "1", author: ChatAuthor(id: author, name: "Author", isGuest: false),
                    content: ChatContent(version: 1, type: "text", text: text, mentions: mentions), createdAt: "now", clientMessageId: "c")
    }

    func testFeedbackAndCaperTokensArePillsWithoutCards() {
        let mentions: [MessageMention] = [.feedback, .user(id: "Bob000000001", username: "bob")]
        XCTAssertEqual(MentionAutocomplete.segments(in: "@Feedback and @caper, ask @bob or feedback@caper.chat @feedbacks", mentions: mentions), [
            MentionSegment(text: "@Feedback", highlighted: true),
            MentionSegment(text: " and ", highlighted: false),
            MentionSegment(text: "@caper", highlighted: true),
            MentionSegment(text: ", ask ", highlighted: false),
            MentionSegment(text: "@bob", highlighted: true, user: MentionPill(id: "Bob000000001", username: "bob")),
            MentionSegment(text: " or feedback@caper.chat @feedbacks", highlighted: false),
        ])
        XCTAssertEqual(MentionAutocomplete.pills(in: "@feedback @bob", mentions: mentions), [MentionPill(id: "Bob000000001", username: "bob")],
                       "@feedback pills open no profile card")
        XCTAssertEqual(MentionAutocomplete.segments(in: "@feedback", mentions: [.everyone]), [MentionSegment(text: "@feedback", highlighted: false)],
                       "without the feedback entry (a guest's text) it stays plain")
        // Even a user who happens to be named caper: the alias wins when the message asked the team.
        XCTAssertNil(MentionAutocomplete.segments(in: "@caper", mentions: [.feedback, .user(id: "Caper0000001", username: "caper")]).first?.user)

        XCTAssertTrue(MentionAutocomplete.asksFeedback(message("@caper hi", [.feedback])))
        XCTAssertFalse(MentionAutocomplete.asksFeedback(message("@caper hi", [])))
        XCTAssertFalse(MentionAutocomplete.mentionsCurrentUser(message("@feedback", [.feedback]), currentUserID: selfID),
                       "@feedback does not tint the row as a mention of you")
        XCTAssertEqual(FeedbackInbox.notified, "Caper team has been notified 🫡")
        // Every team name is a pill once the message asked the team.
        XCTAssertEqual(MentionAutocomplete.segments(in: "@Admin @mods @support_x", mentions: [.feedback]), [
            MentionSegment(text: "@Admin", highlighted: true),
            MentionSegment(text: " ", highlighted: false),
            MentionSegment(text: "@mods", highlighted: true),
            MentionSegment(text: " @support_x", highlighted: false),
        ])
        for name in MentionAutocomplete.feedbackAliases {
            XCTAssertTrue(MentionAutocomplete.isHighlighted(name: name.uppercased(), mentions: [.feedback]), name)
        }
    }

    // MARK: Autocomplete

    private func caret(_ text: String) -> NSRange { NSRange(location: (text as NSString).length, length: 0) }
    private func names(_ text: String, _ source: MentionSource) -> [String] {
        MentionAutocomplete.match(text: text, selection: caret(text), markedText: false, source: source)?.choices.map(\.username) ?? []
    }

    func testFeedbackSuggestionMatchesEitherAliasEverywhere() throws {
        XCTAssertEqual(MentionCandidate.feedback.kind, .feedback)
        XCTAssertEqual(MentionCandidate.feedback.displayName, "Notify the Caper team")
        XCTAssertEqual(ComposerSuggestionsView.mentionLabel(.feedback), "@feedback, Notify the Caper team")
        XCTAssertEqual(MentionAutocomplete.feedbackAliases.first, "feedback")
        XCTAssertEqual(MentionAutocomplete.feedbackAliases.count, 17)

        let members = [Member(id: "Carol0000001", username: "carol", displayName: "Carol King", owner: false)]
        let space = MentionSource.space(members: members, excluding: selfID)
        XCTAssertEqual(names("@f", space), ["feedback"])
        XCTAssertEqual(names("@FEED", space), ["feedback"])
        XCTAssertEqual(names("@feedback", space), ["feedback"])
        XCTAssertEqual(names("@ca", space), ["carol", "caper"], "the first team name the query starts, after people")
        XCTAssertEqual(names("@caper", space), ["caper"])
        XCTAssertEqual(names("@capers", space), ["capers"])
        XCTAssertEqual(names("@caper_s", space), ["caper_support"])
        XCTAssertEqual(names("@ad", space), ["admin"])
        XCTAssertEqual(names("@mo", space), ["mod"])
        XCTAssertEqual(names("@s", space), ["support"])
        XCTAssertEqual(names("@st", space), ["staff"])
        XCTAssertEqual(names("@TEA", space), ["team"])
        XCTAssertEqual(names("@feedbackx", space), [])
        XCTAssertEqual(names("@moderatorsx", space), [])
        let admin = try XCTUnwrap(MentionAutocomplete.match(text: "@ad", selection: caret("@ad"), markedText: false, source: space))
        XCTAssertEqual(admin.choices.map(\.kind), [.feedback], "one team suggestion")
        XCTAssertEqual(MentionAutocomplete.inserting(admin.choices[0], in: "@ad", match: admin)?.text, "@admin ")

        let peer = DirectMessagePeer(id: "Pat000000001", username: "pat", displayName: "Pat")
        XCTAssertEqual(names("@c", .direct(peer: peer, accountID: selfID)), ["caper"], "DMs")
        let notes = MentionSource.direct(peer: DirectMessagePeer(id: selfID, username: "me", displayName: "Me"), accountID: selfID)
        XCTAssertEqual(names("@c", notes), ["caper"], "personal notes")
        XCTAssertEqual(names("@", notes), [], "a bare @ stays about people")
        XCTAssertEqual(names("@", space), ["carol", "everyone", "here"], "no @feedback until something is typed")

        let match = try XCTUnwrap(MentionAutocomplete.match(text: "hi @cap", selection: caret("hi @cap"), markedText: false, source: notes))
        XCTAssertEqual(MentionAutocomplete.inserting(match.choices[0], in: "hi @cap", match: match)?.text, "hi @caper ",
                       "the matching team name is inserted")
    }

    func testVisitSuggestsTheReporterAndPeopleInTheConversation() {
        let reporter = FeedbackReport.Reporter(id: "Alice0000001", username: "alice", displayName: "Alice", avatarId: 12)
        let people = [Person(id: "Bob000000001", username: "bob", displayName: "Bob Stone")]
        let messages = [
            message("hi", [], author: "Bob000000001"),
            message("hey @dana", [.user(id: "Dana00000001", username: "dana")], author: "Stranger0001"),
            message("me", [.user(id: selfID, username: "team")], author: selfID),
            message("again", [], author: "Alice0000001"),
        ]
        let source = MentionSource.visit(reporter: reporter, messages: messages, people: people, accountID: selfID)
        XCTAssertFalse(source.specials, "no @everyone/@here in a visit")
        XCTAssertTrue(source.feedback)
        XCTAssertEqual(source.members.map(\.username), ["alice", "bob", "dana"],
                       "the reporter, named authors and resolved mentions, never yourself, without duplicates")
        XCTAssertEqual(source.members.first?.avatarId, 12)
        XCTAssertEqual(names("@", source), ["alice", "bob", "dana"])
        XCTAssertEqual(names("@f", source), ["feedback"])
        XCTAssertEqual(names("@f", .visit(reporter: nil, messages: [], people: nil, accountID: selfID)), ["feedback"],
                       "otherwise only @feedback")
    }

    // MARK: Inbox presentation

    func testInboxRowCopy() throws {
        let page = try JSONDecoder().decode(FeedbackReportsPage.self, from: Data(samplePage.utf8))
        let channel = page.reports[0], direct = page.reports[1]
        XCTAssertEqual(FeedbackInbox.location(channel), "#plans · Studio")
        XCTAssertEqual(FeedbackInbox.location(direct), "DM · Alice & Bob")
        XCTAssertEqual(FeedbackInbox.preview(channel), "@feedback uploads fail from the share sheet", "one line")
        XCTAssertEqual(FeedbackInbox.preview(direct), "Sent 1 file")
        let files = FeedbackReport(id: "x", messageId: "x", threadRootId: "x", createdAt: "", text: "  ", attachments: 3,
                                   reporter: channel.reporter, channel: channel.channel, space: nil)
        XCTAssertEqual(FeedbackInbox.preview(files), "Sent 3 files")
        XCTAssertEqual(FeedbackInbox.status(channel), "Answered")
        XCTAssertEqual(FeedbackInbox.status(direct), "Waiting")
        XCTAssertTrue(FeedbackInbox.accessibilityLabel(channel, archived: false).hasSuffix("Answered"))
        XCTAssertFalse(FeedbackInbox.accessibilityLabel(channel, archived: true).contains("Answered"), "archived rows have no status")
        XCTAssertTrue(channel.sharesThread(with: channel))
        XCTAssertFalse(channel.sharesThread(with: direct))
        XCTAssertEqual(FeedbackInbox.archivedTitle, "Archived feedback")
        XCTAssertEqual(FeedbackInbox.archivedEmpty, "Nothing archived.")
        XCTAssertEqual(FeedbackInbox.showArchived, "Show archived")
        XCTAssertEqual(FeedbackInbox.backToFeedback, "Back to feedback")
        XCTAssertEqual(FeedbackInbox.restore, "Restore to the inbox")
        XCTAssertEqual(FeedbackInbox.waitingCount(page.reports), 1)
        XCTAssertEqual(FeedbackInbox.merging([channel], [channel, direct]).map(\.id), [channel.id, direct.id], "pages never repeat a report")

        let created = try XCTUnwrap(FeedbackInbox.parseDate("2026-10-11T19:00:00.123456+00:00"), "microseconds from Postgres")
        let whole = try XCTUnwrap(ISO8601DateFormatter().date(from: "2026-10-11T19:00:00Z"))
        XCTAssertEqual(created.timeIntervalSince1970, whole.timeIntervalSince1970, accuracy: 1)
        XCTAssertEqual(FeedbackInbox.relativeTime("2026-10-11T19:00:00Z", now: created.addingTimeInterval(20)), "now")
        XCTAssertEqual(FeedbackInbox.relativeTime("2026-10-11T19:00:00Z", now: created.addingTimeInterval(5 * 60)), "5m")
        XCTAssertEqual(FeedbackInbox.relativeTime("2026-10-11T19:00:00Z", now: created.addingTimeInterval(3 * 3_600)), "3h")
        XCTAssertEqual(FeedbackInbox.relativeTime("2026-10-11T19:00:00Z", now: created.addingTimeInterval(2 * 86_400)), "2d")
        XCTAssertEqual(FeedbackInbox.relativeTime("garbage"), "")
    }

    func testTargetsAndVisitTitles() throws {
        let page = try JSONDecoder().decode(FeedbackReportsPage.self, from: Data(samplePage.utf8))
        let reply = page.reports[0].target
        XCTAssertEqual(reply.channelID, channelID)
        XCTAssertEqual(reply.spaceID, "Space0000001")
        XCTAssertEqual(reply.rootID, rootID)
        XCTAssertTrue(reply.isReply)
        XCTAssertEqual(reply.reporter?.username, "alice")
        let topLevel = FeedbackTarget(channelID: channelID, spaceID: "", messageID: rootID, threadRootID: nil)
        XCTAssertNil(topLevel.spaceID, "an empty space is a DM")
        XCTAssertEqual(topLevel.rootID, rootID, "a top-level report opens its own thread")
        XCTAssertFalse(topLevel.isReply)

        XCTAssertEqual(FeedbackVisit(channelID: channelID, spaceID: "Space0000001", spaceName: "Studio", channelName: "Plans",
                                     direct: false, reportID: rootID).title, "#plans · Studio")
        XCTAssertEqual(FeedbackVisit(channelID: "Dm0000000001", spaceID: "", spaceName: "Direct messages", channelName: "Alice & Bob",
                                     direct: true, reportID: rootID).title, "Alice & Bob")
    }

    // MARK: Notifications

    func testFeedbackNotificationsRouteToAVisit() {
        let channel: [AnyHashable: Any] = ["kind": "feedback", "messageId": replyID, "threadRootId": rootID,
                                           "spaceId": "Space0000001", "channelId": channelID, "title": "Alice · Feedback"]
        XCTAssertEqual(NotificationRoute(userInfo: channel),
                       .feedback(FeedbackTarget(channelID: channelID, spaceID: "Space0000001", messageID: replyID, threadRootID: rootID)))
        let direct: [AnyHashable: Any] = ["kind": "feedback", "messageId": rootID, "conversationId": "Dm0000000001"]
        guard case let .feedback(target)? = NotificationRoute(userInfo: direct) else { return XCTFail("a DM feedback notification visits") }
        XCTAssertEqual(target.channelID, "Dm0000000001")
        XCTAssertNil(target.spaceID)
        XCTAssertEqual(target.rootID, rootID, "without threadRootId the message is its own thread")
        XCTAssertEqual(NotificationRoute(userInfo: ["kind": "feedback", "conversationId": "Dm0000000001"]),
                       .direct(conversationID: "Dm0000000001"), "without a message it opens the conversation")
        XCTAssertEqual(NotificationRoute(userInfo: ["kind": "mention.user", "messageId": replyID, "threadRootId": rootID,
                                                    "spaceId": "Space0000001", "channelId": channelID]),
                       .channel(spaceID: "Space0000001", channelID: channelID), "other kinds keep their routes")

        let route = NotificationRoute.feedback(FeedbackTarget(channelID: channelID, spaceID: "Space0000001", messageID: replyID, threadRootID: rootID))
        XCTAssertTrue(route.isOpen(spaceID: "Space0000001", channelID: nil, directMessageID: nil, visitChannelID: channelID))
        XCTAssertTrue(route.isOpen(spaceID: "Space0000001", channelID: channelID, directMessageID: nil), "opened the normal way")
        XCTAssertFalse(route.isOpen(spaceID: "Space0000001", channelID: nil, directMessageID: nil))
        XCTAssertFalse(route.isOpen(spaceID: "Space0000001", channelID: channelID, directMessageID: "Dm0000000001"))
    }

    // MARK: Model

    @MainActor
    func testInboxLoadsPagesAndHidesWhenNoLongerOnTheTeam() async throws {
        var pages: [String] = []
        var status = 200
        FeedbackMockURLProtocol.handler = { request in
            guard request.url?.path == "/api/feedback" else { throw URLError(.badURL) }
            pages.append(request.url?.query ?? "")
            if status != 200 { return (status, Data(#"{"error":"not found"}"#.utf8)) }
            if request.url?.query == "before=123" {
                return (200, Data(#"{"reports":[],"cursor":null}"#.utf8))
            }
            return (200, Data(self.samplePage.utf8))
        }
        let model = AppModel(api: client())
        model.account = Account(id: "Plain0000001", username: "plain", displayName: "Plain")
        await model.refreshFeedback()
        XCTAssertEqual(pages, [], "only the team asks for the inbox")

        model.account = Account(id: selfID, username: "team", displayName: "Team", feedbackTeam: true)
        await model.refreshFeedback()
        XCTAssertEqual(model.feedbackReports.count, 2)
        XCTAssertEqual(model.feedbackCursor, "123")
        XCTAssertTrue(model.feedbackLoaded)
        await model.loadMoreFeedback()
        XCTAssertEqual(pages, ["", "before=123"])
        XCTAssertEqual(model.feedbackReports.count, 2)
        XCTAssertNil(model.feedbackCursor)

        status = 404
        model.showingFeedback = true
        await model.refreshFeedback()
        XCTAssertFalse(model.isFeedbackTeam, "a 404 means the account left the team")
        XCTAssertEqual(model.feedbackReports, [])
        XCTAssertFalse(model.showingFeedback)
    }

    @MainActor
    func testArchivingRemovesTheThreadAtOnceAndArchivedListSwitches() async throws {
        var requests: [String] = []
        var bodies: [[String: Any]] = []
        var archiveStatus = 204
        FeedbackMockURLProtocol.handler = { request in
            let line = "\(request.httpMethod ?? "") \(request.url?.path ?? "")" + (request.url?.query.map { "?\($0)" } ?? "")
            requests.append(line)
            switch request.url?.path {
            case "/api/feedback":
                if request.url?.query?.contains("archived=true") == true {
                    return (200, Data(#"{"reports":[],"cursor":null}"#.utf8))
                }
                return (200, Data(self.samplePage.utf8))
            case "/api/feedback/archive":
                bodies.append(try XCTUnwrap(JSONSerialization.jsonObject(with: feedbackBody(request)) as? [String: Any]))
                return (archiveStatus, archiveStatus == 204 ? Data() : Data(#"{"error":"unavailable"}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let model = AppModel(api: client())
        model.account = Account(id: selfID, username: "team", displayName: "Team", feedbackTeam: true)
        await model.refreshFeedback()
        let report = try XCTUnwrap(model.feedbackReports.first)
        XCTAssertEqual(FeedbackInbox.archiveLabel(report), "Archive feedback from Alice")

        requests = []
        await model.setFeedbackArchived(report, archived: true)
        XCTAssertEqual(requests, ["PUT /api/feedback/archive", "GET /api/feedback"], "archives, then refreshes")
        XCTAssertEqual(bodies.first?["channelId"] as? String, channelID)
        XCTAssertEqual(bodies.first?["threadRootId"] as? String, rootID)
        XCTAssertEqual(bodies.first?["archived"] as? Bool, true)
        XCTAssertNil(model.feedbackError)

        archiveStatus = 503
        await model.setFeedbackArchived(report, archived: true)
        XCTAssertEqual(model.feedbackError, FeedbackInbox.archiveFailed, "a failure shows why")
        XCTAssertEqual(model.feedbackReports.count, 2, "and reloads the list")

        requests = []
        await model.showArchivedFeedback(true)
        XCTAssertTrue(model.showingArchivedFeedback)
        XCTAssertEqual(requests, ["GET /api/feedback?archived=true"])
        XCTAssertEqual(model.feedbackReports, [])
        XCTAssertTrue(model.feedbackLoaded)
        XCTAssertNil(model.feedbackError)
        archiveStatus = 204
        await model.setFeedbackArchived(report, archived: false)
        XCTAssertEqual(bodies.last?["archived"] as? Bool, false, "Restore to the inbox")
        await model.showArchivedFeedback(false)
        XCTAssertEqual(requests.last, "GET /api/feedback")
        XCTAssertEqual(model.feedbackReports.count, 2)

        // The API keeps archived and paging together.
        requests = []
        _ = try await client().feedbackReports(before: "123", archived: true)
        XCTAssertEqual(requests, ["GET /api/feedback?archived=true&before=123"])
    }

    @MainActor
    func testOpeningAReportVisitsWithItsThreadOpen() async throws {
        let history = """
        {"space":{"id":"Space0000001","name":"Studio"},"channel":{"id":"\(channelID)","name":"plans"},
         "messages":[{"id":"\(rootID)","channelId":"\(channelID)","seq":"5","author":{"id":"Alice0000001","name":"Alice","isGuest":false},
           "content":{"version":1,"type":"text","text":"uploads fail"},"createdAt":"now","clientMessageId":"r",
           "thread":{"replyCount":1,"participants":[],"seq":"6"}}],
         "pinnedMessages":[],"cursor":"6","hasMore":true,"hasNewer":false,"visitor":true}
        """
        let thread = """
        {"root":{"id":"\(rootID)","channelId":"\(channelID)","seq":"5","author":{"id":"Alice0000001","name":"Alice","isGuest":false},
           "content":{"version":1,"type":"text","text":"uploads fail"},"createdAt":"now","clientMessageId":"r"},
         "messages":[{"id":"\(replyID)","channelId":"\(channelID)","seq":"6","author":{"id":"Alice0000001","name":"Alice","isGuest":false},
           "content":{"version":1,"type":"text","text":"@feedback help","mentions":[{"type":"feedback"}]},"createdAt":"now",
           "clientMessageId":"q","threadRootId":"\(rootID)"}],
         "cursor":"6","hasMore":false,"hasNewer":false,"visitor":true}
        """
        var paths: [String] = []
        FeedbackMockURLProtocol.handler = { request in
            let path = request.url?.path ?? ""
            paths.append(path + (request.url?.query.map { "?\($0)" } ?? ""))
            switch path {
            case "/api/chat/channels/\(self.channelID)/messages": return (200, Data(history.utf8))
            case "/api/chat/channels/\(self.channelID)/messages/\(self.rootID)/thread": return (200, Data(thread.utf8))
            case "/api/chat/session": return (200, Data(#"{"token":"chat-secret","author":{"id":"Team00000001","name":"Team","isGuest":false}}"#.utf8))
            case "/api/people": return (200, Data(#"{"people":[]}"#.utf8))
            case "/api/channels/\(self.channelID)/media/status": return (200, Data(#"{"enabled":true}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let model = AppModel(api: client())
        model.account = Account(id: selfID, username: "team", displayName: "Team", feedbackTeam: true)
        let reporter = FeedbackReport.Reporter(id: "Alice0000001", username: "alice", displayName: "Alice")
        await model.openFeedback(FeedbackTarget(channelID: channelID, spaceID: "Space0000001", messageID: replyID,
                                                threadRootID: rootID, reporter: reporter))

        XCTAssertEqual(paths.first, "/api/chat/channels/\(channelID)/messages?around=\(rootID)", "a reply loads around its thread root")
        XCTAssertEqual(model.visit?.channelID, channelID)
        XCTAssertEqual(model.visit?.title, "#plans · Studio")
        XCTAssertEqual(model.visit?.reporter, reporter)
        XCTAssertNil(model.selectedChannelID, "a visit is not a channel in the sidebar")
        XCTAssertNil(model.selectedDirectMessageID)
        XCTAssertTrue(model.showingFeedback, "Back returns to the inbox")
        let channel = try XCTUnwrap(model.visitChannel, "a visited space channel offers voice")
        XCTAssertEqual(channel.spaceId, "Space0000001")
        XCTAssertNil(model.voiceAvailable(in: channel))
        await model.refreshVoiceAvailability(channel: channel)
        XCTAssertEqual(model.voiceAvailable(in: channel), true)
        XCTAssertEqual(model.voiceRoster(for: channelID), [])
        XCTAssertEqual(model.chat.threadRootID, rootID, "replies land in the report's thread")
        XCTAssertEqual(model.chat.focusedMessageID, replyID)
        XCTAssertTrue(model.chat.threadMessages.contains { $0.id == replyID })
        XCTAssertTrue(model.isShowing(.feedback(FeedbackTarget(channelID: channelID, spaceID: nil, messageID: replyID, threadRootID: nil))))
        XCTAssertFalse(paths.contains { $0.contains("/read") }, "no read markers for visitors")

        await model.closeVisit()
        XCTAssertNil(model.visit, "with nothing before it, closing leaves an empty stage")
        XCTAssertNil(model.visitChannel)
    }
}
