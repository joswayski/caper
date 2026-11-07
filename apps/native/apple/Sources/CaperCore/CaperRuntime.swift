import Foundation

@MainActor public enum CaperRuntime {
    static func showWhatsNewPreview(_ model: AppModel, environment: [String: String] = ProcessInfo.processInfo.environment) {
        guard environment["CAPER_TEST_MODE"] == "parity", environment["CAPER_UI_FIXTURE"] == "whats-new",
              let rawURL = environment["CAPER_API_BASE_URL"], let url = URL(string: rawURL),
              ["localhost", "127.0.0.1", "::1"].contains(url.host?.lowercased() ?? "") else { return }
        model.whatsNew.presentation = WhatsNewDisplay(sequence: nil, releases: [
            PlatformRelease(sequence: 3, revision: String(repeating: "a", count: 40), publishedAt: "2026-10-11T19:00:00Z", highlights: [
                ReleaseHighlight(title: "TEST FIXTURE — Message editing", body: "This local preview shows curated release highlights. It does not claim a shipped update."),
            ]),
        ], automatic: false)
    }

    static func isChatPreview(_ name: String, environment: [String: String] = ProcessInfo.processInfo.environment) -> Bool {
        guard ["chat-rejected", "reaction-chips"].contains(name), environment["CAPER_TEST_MODE"] == "parity",
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

    /// Local call rosters: `voice-roster` (you and Maya in the open channel),
    /// `voice-crowd` (seven people, Alex speaking) and `voice-elsewhere` (you
    /// and Maya in another space's channel, which has no row here).
    static let voiceRosterFixtures: Set<String> = ["voice-roster", "voice-crowd", "voice-elsewhere"]

    public static func showVoiceRosterPreview(_ model: AppModel, environment: [String: String] = ProcessInfo.processInfo.environment) {
        guard environment["CAPER_TEST_MODE"] == "parity", let fixture = environment["CAPER_UI_FIXTURE"],
              voiceRosterFixtures.contains(fixture),
              let rawURL = environment["CAPER_API_BASE_URL"], let baseURL = URL(string: rawURL),
              ["localhost", "127.0.0.1", "::1"].contains(baseURL.host?.lowercased() ?? ""),
              model.voice.context == nil,
              let detail = model.detail,
              let channel = detail.channels.first(where: { $0.id == model.selectedChannelID }) else { return }
        let crowd = fixture == "voice-crowd" ? """
        ,{"id":"fixture-crowd-alex","name":"TEST FIXTURE Alex","muted":false,"deafened":false,"tracks":[]},
         {"id":"fixture-crowd-sam","name":"TEST FIXTURE Sam","muted":false,"deafened":true,"tracks":[]},
         {"id":"fixture-crowd-jo","name":"TEST FIXTURE Jo","muted":false,"deafened":false,"tracks":[]},
         {"id":"fixture-crowd-ri","name":"TEST FIXTURE Ri","muted":false,"deafened":false,"tracks":[]},
         {"id":"fixture-crowd-kai","name":"TEST FIXTURE Kai","muted":false,"deafened":false,"tracks":[]}
        """ : ""
        let data = Data("""
        [{"id":"fixture-self","name":"TEST FIXTURE You","muted":false,"deafened":false,"tracks":[]},
         {"id":"fixture-remote","name":"TEST FIXTURE Maya","muted":true,"deafened":false,"tracks":[]}\(crowd)]
        """.utf8)
        guard let participants = try? JSONDecoder().decode([VoiceParticipant].self, from: data) else { return }
        let context = fixture == "voice-elsewhere"
            ? VoiceContext(channelID: "fixture-elsewhere-lobby", channelName: "lobby",
                           spaceID: "fixture-elsewhere-space", spaceName: "TEST FIXTURE Elsewhere")
            : VoiceContext(channelID: channel.id, channelName: channel.name,
                           spaceID: detail.space.id, spaceName: detail.space.name)
        model.voice.displayRosterPreview(context: context, selfID: "fixture-self", participants: participants,
                                         speaking: fixture == "voice-crowd" ? ["fixture-crowd-alex"] : [])
    }

    /// The fixture server this launch talks to, or nil for caper.chat.
    /// Shipping builds accept only a loopback fixture in a parity launch.
    /// Bench builds (`CAPER_DEVICE_FIXTURE`, device-fixture-project.yml) also
    /// accept a private-LAN fixture and never fall back: missing or invalid
    /// settings stop the app rather than reach caper.chat.
    nonisolated public static func fixtureBaseURL(environment: [String: String] = ProcessInfo.processInfo.environment) -> URL? {
        #if CAPER_DEVICE_FIXTURE
        switch FixtureServer.resolve(environment, privateNetwork: true) {
        case .success(let url): return url
        case .failure(let rejection):
            fatalError("TEST FIXTURE bench build: \(rejection). It never connects to caper.chat.")
        }
        #else
        return try? FixtureServer.resolve(environment, privateNetwork: false).get()
        #endif
    }

    public static func makeModel(environment: [String: String] = ProcessInfo.processInfo.environment) -> AppModel {
        guard let baseURL = fixtureBaseURL(environment: environment) else { return AppModel() }
        // Fixture sessions stay in memory, never in the Keychain.
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

/// Which fixture server a parity launch (`CAPER_TEST_MODE=parity`,
/// `CAPER_API_BASE_URL`) may use. Loopback hosts are always allowed. Bench
/// builds for a physical iPhone also allow plain-HTTP private-LAN IPv4
/// literals (10/8, 172.16/12, 192.168/16): a host name could resolve anywhere.
enum FixtureServer {
    enum Rejection: Error, Equatable, CustomStringConvertible {
        case notParity, missingURL, invalidURL(String), notFixtureHost(String)

        var description: String {
            switch self {
            case .notParity: return "CAPER_TEST_MODE is not parity"
            case .missingURL: return "CAPER_API_BASE_URL is missing"
            case .invalidURL(let raw): return "CAPER_API_BASE_URL \(raw) is not a plain http://host:port fixture URL"
            case .notFixtureHost(let host): return "\(host) is not a loopback or private-LAN IPv4 fixture host"
            }
        }
    }

    static let loopbackHosts: Set<String> = ["localhost", "127.0.0.1", "::1"]

    static func resolve(_ environment: [String: String], privateNetwork: Bool) -> Result<URL, Rejection> {
        guard environment["CAPER_TEST_MODE"] == "parity" else { return .failure(.notParity) }
        guard let raw = environment["CAPER_API_BASE_URL"], !raw.isEmpty else { return .failure(.missingURL) }
        guard let url = URL(string: raw), let host = url.host?.lowercased() else { return .failure(.invalidURL(raw)) }
        guard privateNetwork else {
            return loopbackHosts.contains(host) ? .success(url) : .failure(.notFixtureHost(host))
        }
        guard loopbackHosts.contains(host) || isPrivateIPv4(host) else { return .failure(.notFixtureHost(host)) }
        guard url.scheme == "http", url.user == nil, url.password == nil, url.query == nil, url.fragment == nil,
              ["", "/"].contains(url.path) else { return .failure(.invalidURL(raw)) }
        return .success(url)
    }

    static func isPrivateIPv4(_ host: String) -> Bool {
        let parts = host.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 4 else { return false }
        let octets = parts.compactMap { part -> Int? in
            guard (1...3).contains(part.count), part.allSatisfy(\.isASCII), part.allSatisfy(\.isNumber),
                  part == "0" || !part.hasPrefix("0"), let value = Int(part), value <= 255 else { return nil }
            return value
        }
        guard octets.count == 4 else { return false }
        return octets[0] == 127 || octets[0] == 10 || (octets[0] == 172 && (16...31).contains(octets[1]))
            || (octets[0] == 192 && octets[1] == 168)
    }
}
