import Foundation

@MainActor enum CaperRuntime {
    static func isChatPreview(_ name: String, environment: [String: String] = ProcessInfo.processInfo.environment) -> Bool {
        guard name == "chat-rejected", environment["CAPER_TEST_MODE"] == "parity",
              environment["CAPER_UI_FIXTURE"] == name,
              let rawURL = environment["CAPER_API_BASE_URL"], let baseURL = URL(string: rawURL),
              ["localhost", "127.0.0.1", "::1"].contains(baseURL.host?.lowercased() ?? "") else { return false }
        return true
    }

    static func isAudioPreview(_ name: String, environment: [String: String] = ProcessInfo.processInfo.environment) -> Bool {
        guard ["audio-recorded", "audio-statistics"].contains(name),
              environment["CAPER_TEST_MODE"] == "parity",
              environment["CAPER_UI_FIXTURE"] == name,
              let rawURL = environment["CAPER_API_BASE_URL"],
              let baseURL = URL(string: rawURL),
              ["localhost", "127.0.0.1", "::1"].contains(baseURL.host?.lowercased() ?? "") else { return false }
        return true
    }

    static func showVoiceRosterPreview(_ model: AppModel, environment: [String: String] = ProcessInfo.processInfo.environment) {
        guard environment["CAPER_TEST_MODE"] == "parity", environment["CAPER_UI_FIXTURE"] == "voice-roster",
              let rawURL = environment["CAPER_API_BASE_URL"], let baseURL = URL(string: rawURL),
              ["localhost", "127.0.0.1", "::1"].contains(baseURL.host?.lowercased() ?? ""),
              model.voice.context == nil,
              let detail = model.detail,
              let channel = detail.channels.first(where: { $0.id == model.selectedChannelID }) else { return }
        let data = Data("""
        [{"id":"fixture-self","name":"TEST FIXTURE You","muted":false,"deafened":false,"tracks":[]},
         {"id":"fixture-remote","name":"TEST FIXTURE Maya","muted":true,"deafened":false,"tracks":[]}]
        """.utf8)
        guard let participants = try? JSONDecoder().decode([VoiceParticipant].self, from: data) else { return }
        model.voice.displayRosterPreview(
            context: VoiceContext(channelID: channel.id, channelName: channel.name,
                                  spaceID: detail.space.id, spaceName: detail.space.name),
            selfID: "fixture-self", participants: participants
        )
    }

    static func makeModel(environment: [String: String] = ProcessInfo.processInfo.environment) -> AppModel {
        guard environment["CAPER_TEST_MODE"] == "parity",
              let rawURL = environment["CAPER_API_BASE_URL"],
              let baseURL = URL(string: rawURL),
              ["localhost", "127.0.0.1", "::1"].contains(baseURL.host?.lowercased() ?? "") else {
            return AppModel()
        }
        let store = FixtureTokenStore(token: environment["CAPER_TEST_BEARER"])
        let model = AppModel(
            api: APIClient(baseURL: baseURL, tokenStore: store),
            preferredInitialSpaceID: environment["CAPER_TEST_SPACE_ID"]
        )
        if environment["CAPER_UI_FIXTURE"] == "profile-validation" {
            model.phase = .onboarding
            model.error = "TEST FIXTURE — username already taken. Choose another username."
        }
        return model
    }
}

private final class FixtureTokenStore: TokenStore, @unchecked Sendable {
    private let lock = NSLock()
    private var token: String?
    init(token: String?) { self.token = token }
    func load() throws -> String? { lock.withLock { token } }
    func save(_ token: String) throws { lock.withLock { self.token = token } }
    func clear() throws { lock.withLock { token = nil } }
}
