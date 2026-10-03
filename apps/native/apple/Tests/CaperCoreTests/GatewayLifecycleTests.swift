import XCTest
@testable import CaperCore

private actor TokenRequestCounter {
    private(set) var count = 0
    let started: XCTestExpectation
    private var requests: [CheckedContinuation<String?, Never>] = []

    init(started: XCTestExpectation) { self.started = started }

    func fetch() async -> String? {
        count += 1
        if count == 1 { started.fulfill() }
        return await withCheckedContinuation { requests.append($0) }
    }

    func finish() {
        for request in requests { request.resume(returning: nil) }
        requests = []
    }
}

@MainActor
final class GatewayLifecycleTests: XCTestCase {
    func testSequentialSubscriptionsSharePendingConnection() async {
        let started = expectation(description: "authorization is pending")
        let tokens = TokenRequestCounter(started: started)
        var states: [GatewayState] = []
        let gateway = Gateway(
            baseURL: URL(string: "https://caper.invalid")!,
            token: { await tokens.fetch() },
            state: { state, _ in states.append(state) }
        )

        _ = await gateway.subscribeMedia(channelID: "one", token: nil) { _ in }
        // Force the race's boundary: the first task owns token lookup but has
        // not assigned socket. A second subscription must not replace it.
        await fulfillment(of: [started], timeout: 1)
        _ = await gateway.subscribeMedia(channelID: "two", token: nil) { _ in }
        _ = await gateway.subscribeMedia(channelID: "three", token: nil) { _ in }
        try? await Task.sleep(for: .milliseconds(50))

        let tokenRequests = await tokens.count
        XCTAssertEqual(tokenRequests, 1)
        XCTAssertEqual(states, [.connecting])
        XCTAssertFalse(states.contains(.reconnecting), "pending token lookup is not reconnect backoff")
        await gateway.stop()
        await tokens.finish()
    }
}
