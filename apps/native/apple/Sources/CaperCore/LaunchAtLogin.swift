#if os(macOS)
import AppKit
import Observation
import ServiceManagement
import SwiftUI

/// Read the OS registration, never a cached UserDefaults preference. In
/// particular, returning from System Settings must not restore revoked consent.
@MainActor @Observable public final class CaperLoginItem {
    public static let shared = CaperLoginItem(
        available: ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] != "parity",
        status: { SMAppService.mainApp.status },
        register: { try SMAppService.mainApp.register() },
        unregister: { try SMAppService.mainApp.unregister() }
    )

    public private(set) var status: SMAppService.Status = .notRegistered
    public var error: String?
    public let available: Bool
    private let readStatus: () -> SMAppService.Status
    private let register: () throws -> Void
    private let unregister: () throws -> Void

    init(available: Bool, status: @escaping () -> SMAppService.Status,
         register: @escaping () throws -> Void, unregister: @escaping () throws -> Void) {
        self.available = available
        readStatus = status
        self.register = register
        self.unregister = unregister
        refresh()
    }

    /// A checked toggle means registered. Approval-required is explicitly shown
    /// beside it and can still be cancelled by switching the toggle off.
    public var registered: Bool { status == .enabled || status == .requiresApproval }

    public func refresh() {
        guard available else { return }
        status = readStatus()
    }

    public func setEnabled(_ enabled: Bool) {
        guard available else { return }
        refresh()
        error = nil
        do {
            if enabled && !registered { try register() }
            else if !enabled && registered { try unregister() }
        } catch {
            self.error = "Could not change startup settings: \(error.localizedDescription)"
        }
        refresh()
    }
}

/// Shared by the account menu and the macOS app menu (also while signed out).
@MainActor public struct CaperLaunchAtLoginControls: View {
    @State private var loginItem = CaperLoginItem.shared
    public init() {}

    public var body: some View {
        Toggle("Launch at login", isOn: Binding(
            get: { loginItem.registered }, set: { loginItem.setEnabled($0) }
        ))
        .disabled(!loginItem.available)
        .accessibilityIdentifier("launch-at-login")
        if loginItem.status == .requiresApproval {
            Text("Launch at login needs approval in System Settings.")
            Button("Open Login Items Settings…") { SMAppService.openSystemSettingsLoginItems() }
        }
    }
}
#endif
