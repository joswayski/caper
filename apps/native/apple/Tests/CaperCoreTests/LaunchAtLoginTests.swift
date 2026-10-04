#if os(macOS)
import ServiceManagement
import XCTest
@testable import CaperCore

final class LaunchAtLoginTests: XCTestCase {
    @MainActor
    func testOptInAndOptOutUseOSRegistrationWithoutLaunchSideEffects() {
        var status = SMAppService.Status.notRegistered
        var registrations = 0
        var removals = 0
        let item = CaperLoginItem(available: true, status: { status }, register: {
            registrations += 1
            status = .enabled
        }, unregister: {
            removals += 1
            status = .notRegistered
        })
        XCTAssertFalse(item.registered)
        XCTAssertEqual(registrations, 0)
        item.setEnabled(true)
        XCTAssertTrue(item.registered)
        XCTAssertEqual(registrations, 1)
        item.refresh()
        XCTAssertEqual(registrations, 1, "Reading startup settings must not register again")
        item.setEnabled(false)
        XCTAssertFalse(item.registered)
        XCTAssertEqual(removals, 1)
        XCTAssertNil(item.error)
    }

    @MainActor
    func testPendingApprovalAndExternalRevocationAreNotReportedAsEnabled() {
        var status = SMAppService.Status.notRegistered
        var registrations = 0
        let item = CaperLoginItem(available: true, status: { status }, register: {
            registrations += 1
            status = .requiresApproval
        }, unregister: { status = .notRegistered })
        item.setEnabled(true)
        XCTAssertTrue(item.registered)
        XCTAssertEqual(item.status, .requiresApproval)
        status = .enabled
        item.refresh()
        XCTAssertEqual(item.status, .enabled)
        status = .requiresApproval
        item.refresh()
        XCTAssertEqual(item.status, .requiresApproval)
        XCTAssertEqual(registrations, 1, "External revocation must not trigger registration")
        item.setEnabled(false)
        XCTAssertEqual(item.status, .notRegistered, "Pending approval can be cancelled")
    }

    @MainActor
    func testFailedChangesPreserveActualStateAndSurfaceError() {
        var status = SMAppService.Status.notRegistered
        let failure = NSError(domain: "caper.login-item.test", code: 1)
        let item = CaperLoginItem(available: true, status: { status },
                                  register: { throw failure }, unregister: { throw failure })
        item.setEnabled(true)
        XCTAssertFalse(item.registered)
        XCTAssertNotNil(item.error)
        status = .enabled
        item.refresh()
        item.setEnabled(false)
        XCTAssertTrue(item.registered)
        XCTAssertNotNil(item.error)
    }

    @MainActor
    func testFixturesNeverReadOrWriteLoginItems() {
        let item = CaperLoginItem(available: false, status: {
            XCTFail("Fixture read real login items")
            return .enabled
        }, register: { XCTFail("Fixture registered a login item") },
           unregister: { XCTFail("Fixture removed a login item") })
        item.refresh()
        item.setEnabled(true)
        item.setEnabled(false)
        XCTAssertFalse(item.registered)
        XCTAssertNil(item.error)
    }
}
#endif
