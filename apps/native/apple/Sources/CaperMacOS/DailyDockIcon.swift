import AppKit
import CaperCore
import Security

@MainActor
final class DailyDockIcon {
    private var timer: Timer?
    private var currentIndex: Int?
    // NSWorkspace permits background calls, but custom-icon writes must be serial.
    private let finderQueue = DispatchQueue(label: "chat.caper.finder-icon", qos: .utility)

    func start(enabled: Bool) {
        guard enabled, timer == nil else { return }
        refresh()
        NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification,
            object: nil, queue: .main) { [weak self] _ in Task { @MainActor in self?.refresh() } }
        timer = Timer.scheduledTimer(withTimeInterval: 15 * 60, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    func refresh(now: Date = Date()) {
        let index = CaperDailyIcon.current(now: now)
        guard currentIndex != index, let image = CaperDailyIcon.dockImage(for: index) else { return }
        NSApplication.shared.applicationIconImage = image
        currentIndex = index
        let bundleURL = Bundle.main.bundleURL
        finderQueue.async { Self.synchronizeInstalledIcon(image, at: bundleURL) }
    }

    /// Best effort for this running copy only; never rewrite AppIcon/Info.plist,
    /// re-sign the app, request elevated access, or change another installed copy.
    @discardableResult
    nonisolated static func synchronizeInstalledIcon(_ image: NSImage, at bundleURL: URL,
        signatureIsValid: (URL) -> Bool = hasValidSignature) -> Bool {
        guard bundleURL.pathExtension == "app",
              FileManager.default.isWritableFile(atPath: bundleURL.path),
              signatureIsValid(bundleURL) else { return false }
        guard NSWorkspace.shared.setIcon(image, forFile: bundleURL.path, options: []) else { return false }
        guard signatureIsValid(bundleURL) else {
            // Custom-icon metadata varies by OS/filesystem. Remove it if this
            // system's signature validator rejects it, leaving the static icon.
            NSWorkspace.shared.setIcon(nil, forFile: bundleURL.path, options: [])
            return false
        }
        return true
    }

    private nonisolated static func hasValidSignature(at url: URL) -> Bool {
        var code: SecStaticCode?
        guard SecStaticCodeCreateWithPath(url as CFURL, SecCSFlags(), &code) == errSecSuccess,
              let code else { return false }
        let flags = SecCSFlags(rawValue: kSecCSStrictValidate | kSecCSCheckNestedCode | kSecCSCheckAllArchitectures)
        return SecStaticCodeCheckValidity(code, flags, nil) == errSecSuccess
    }
}
