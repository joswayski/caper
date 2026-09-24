import Foundation

@MainActor enum CaperRuntime {
    static func isAudioPreview(_ name: String, environment: [String: String] = ProcessInfo.processInfo.environment) -> Bool {
        guard ["audio-recorded", "audio-statistics"].contains(name),
              environment["CAPER_TEST_MODE"] == "parity",
              environment["CAPER_UI_FIXTURE"] == name,
              let rawURL = environment["CAPER_API_BASE_URL"],
              let baseURL = URL(string: rawURL),
              ["localhost", "127.0.0.1", "::1"].contains(baseURL.host?.lowercased() ?? "") else { return false }
        return true
    }

    static func makeModel(environment: [String: String] = ProcessInfo.processInfo.environment) -> AppModel {
        guard environment["CAPER_TEST_MODE"] == "parity",
              let rawURL = environment["CAPER_API_BASE_URL"],
              let baseURL = URL(string: rawURL),
              ["localhost", "127.0.0.1", "::1"].contains(baseURL.host?.lowercased() ?? "") else {
            return AppModel()
        }
        let store = FixtureTokenStore(token: environment["CAPER_TEST_BEARER"])
        return AppModel(
            api: APIClient(baseURL: baseURL, tokenStore: store),
            preferredInitialSpaceID: environment["CAPER_TEST_SPACE_ID"]
        )
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
