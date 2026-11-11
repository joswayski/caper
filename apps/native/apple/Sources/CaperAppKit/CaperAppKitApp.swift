import AppKit
import CaperCore

@main @MainActor
final class CaperAppKitApp: NSObject, NSApplicationDelegate, NSMenuItemValidation {
    private var window: NSWindow?
    private var model: AppModel?
    private var settings: AppKitSettingsPresenter?
    /// Built under any identity but the development host's, this is the
    /// shipping app: same Keychain session, login item, updater and links.
    private let shipping = Bundle.main.bundleIdentifier != "chat.caper.appkit-development"
    /// The shipping app's updater, or the local update-review fixture's.
    private var updater: AppUpdater?
    /// Links that arrive before launch finishes, such as the one that opened Caper.
    private var pendingLinks: [URL] = []

    static func main() {
        let application = NSApplication.shared
        let delegate = CaperAppKitApp()
        application.delegate = delegate
        application.setActivationPolicy(.regular)
        withExtendedLifetime(delegate) { application.run() }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        CaperFontLoader.register()
        NSApp.appearance = NSAppearance(named: .darkAqua)
        let environment = ProcessInfo.processInfo.environment
        let fixture = environment["CAPER_TEST_MODE"] == "parity"
            && URL(string: environment["CAPER_API_BASE_URL"] ?? "").map {
                ["localhost", "127.0.0.1", "::1"].contains($0.host ?? "")
            } == true
        // The sandboxed UI-test runner cannot clear this app's real defaults
        // domain, so a fixture launch may name keys for the app to clear itself.
        if fixture, let keys = environment["CAPER_TEST_CLEAR_DEFAULTS"] {
            keys.split(separator: ",").forEach { UserDefaults.standard.removeObject(forKey: String($0)) }
        }
        // The development host never reads, overwrites, or revokes the shipping app's session.
        let model = fixture || shipping ? CaperRuntime.makeModel() : AppModel(api: APIClient(
            tokenStore: KeychainTokenStore(service: "chat.caper.appkit-development.session|https://caper.chat")))
        self.model = model
        let updater = shipping || fixture && environment["CAPER_UI_FIXTURE"] == "update" ? AppUpdater() : nil
        self.updater = updater
        // As the SwiftUI app does: a copy that must be installed first (from
        // the disk image) never opens the account UI or starts the session.
        if let updater, !updater.prepareInstallation() { return }
        let settings = shipping
            ? AppKitSettingsPresenter(model: model, loginItem: .shared, updater: updater, title: "Caper Settings")
            : AppKitSettingsPresenter(model: model, updater: updater)
        self.settings = settings
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: fixture ? 1440 : 1180, height: fixture ? 900 : 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = shipping ? "Caper" : "Caper — AppKit development (not feature complete)"
        window.identifier = NSUserInterfaceItemIdentifier(shipping ? "caper-main" : "caper-appkit-development")
        window.contentMinSize = NSSize(width: 390, height: 578)
        window.contentViewController = AppKitRootController(model: model, settings: settings, updater: updater)
        window.setContentSize(NSSize(width: fixture ? 1440 : 1180, height: fixture ? 900 : 760))
        window.center()
        self.window = window
        installMenus()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        pendingLinks.forEach(model.openInviteLink); pendingLinks = []
        Task { await model.start() }
        updater?.start()
    }

    /// caper://invite/<code>, registered only by the shipping Info.plist.
    func application(_ application: NSApplication, open urls: [URL]) {
        guard let model else { pendingLinks += urls; return }
        urls.forEach(model.openInviteLink)
        window?.deminiaturize(nil)
        window?.makeKeyAndOrderFront(nil)
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { window?.makeKeyAndOrderFront(nil) }
        return true
    }

    func applicationDidBecomeActive(_ notification: Notification) { model?.applicationActivityChanged(active: true) }
    func applicationDidResignActive(_ notification: Notification) { model?.applicationActivityChanged(active: false) }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    @objc private func showSettings(_ sender: Any?) { settings?.show(from: window) }
    @objc private func showWhatsNew(_ sender: Any?) { settings?.openWhatsNew(from: window) }
    @objc private func checkForUpdates(_ sender: Any?) { updater?.check(manual: true) }
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if menuItem.action == #selector(showWhatsNew(_:)) { return settings?.canOpenWhatsNew == true }
        if menuItem.action == #selector(checkForUpdates(_:)) { return updater?.isAvailable == true && updater?.checking == false }
        return true
    }

    private func installMenus() {
        let menu = NSMenu()
        let app = NSMenuItem()
        let appMenu = NSMenu()
        if shipping {
            appMenu.addItem(withTitle: "Check for Updates…", action: #selector(checkForUpdates(_:)), keyEquivalent: "").target = self
            appMenu.addItem(.separator())
        }
        appMenu.addItem(withTitle: "Settings…", action: #selector(showSettings(_:)), keyEquivalent: ",").target = self
        appMenu.addItem(withTitle: "What’s new", action: #selector(showWhatsNew(_:)), keyEquivalent: "").target = self
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: shipping ? "Quit Caper" : "Quit Caper AppKit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        app.submenu = appMenu; menu.addItem(app)
        let file = NSMenuItem()
        let fileMenu = NSMenu(title: "File")
        fileMenu.addItem(withTitle: "Close Window", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        file.submenu = fileMenu; menu.addItem(file)
        let edit = NSMenuItem()
        let editMenu = NSMenu(title: "Edit")
        for (title, action, key) in [("Undo", "undo:", "z"), ("Cut", "cut:", "x"), ("Copy", "copy:", "c"),
                                    ("Paste", "paste:", "v"), ("Select All", "selectAll:", "a")] {
            editMenu.addItem(withTitle: title, action: Selector(action), keyEquivalent: key)
        }
        editMenu.addItem(.separator())
        editMenu.addItem(withTitle: "B — Bold", action: #selector(ComposerTextView.formatBold(_:)), keyEquivalent: "b")
        editMenu.addItem(withTitle: "I — Italic", action: #selector(ComposerTextView.formatItalic(_:)), keyEquivalent: "i")
        edit.submenu = editMenu; menu.addItem(edit)
        NSApp.mainMenu = menu
    }
}

@MainActor final class AppKitRootController: NSViewController {
    private let model: AppModel
    private let settings: AppKitSettingsPresenter
    private let updater: AppUpdater?
    private lazy var whatsNew = AppKitWhatsNew.presenter(model: model)
    private let observation = NativeObservation()
    private let updateObservation = NativeObservation()
    private var shownPhase: AppModel.Phase?
    private var review: NSView?

    init(model: AppModel, settings: AppKitSettingsPresenter, updater: AppUpdater?) {
        self.model = model; self.settings = settings; self.updater = updater; super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func loadView() { view = RootView(NativeStyle.blackout) }
    override func viewDidLoad() {
        super.viewDidLoad()
        addChild(whatsNew)
        whatsNew.view.frame = .zero
        view.addSubview(whatsNew.view)
        observation.start(reading: { [model] in model.phase }) { [weak self] in self?.updatePhase($0) }
        if let updater {
            updateObservation.start(reading: { [updater] in (updater.offeredUpdate, updater.reviewRequest) }) { [weak self] in
                self?.showReview($0.0)
            }
        }
    }

    /// As the SwiftUI window does: the review covers the account UI, which
    /// stays inert underneath until it closes.
    private func showReview(_ update: DesktopUpdate?) {
        review?.removeFromSuperview(); review = nil
        (view as? RootView)?.modal = nil
        guard let update, let updater else { return }
        let model = model
        let host = AppKitUpdateReview(update: update,
            inCall: model.voice.phase != .idle && model.voice.phase != .failed,
            close: { updater.dismissReview() },
            restart: {
                if let sequence = update.sequence { model.whatsNew.markRead(sequence) }
                updater.applyReviewedUpdate()
            },
            download: { updater.download() },
            report: model.canSendUpdateReport ? { [installed = updater.installedVersion] error in
                await model.sendUpdateReport(error: error, installed: String(installed.dropFirst("Installed: ".count)),
                                             target: update.version)
            } : nil)
        host.frame = view.bounds
        host.autoresizingMask = [.width, .height]
        view.addSubview(host)
        review = host
        (view as? RootView)?.modal = host
        view.window?.makeFirstResponder(host)
        view.window?.deminiaturize(nil)
        view.window?.makeKeyAndOrderFront(nil)
    }
    private func updatePhase(_ phase: AppModel.Phase) {
        guard shownPhase != phase else { return }
        shownPhase = phase
        for child in children where child !== whatsNew { child.view.removeFromSuperview(); child.removeFromParent() }
        let child: NSViewController
        switch phase {
        case .ready:
            let workspace = WorkspaceController(model: model)
            workspace.updater = updater
            workspace.showSettings = { [weak self] in self?.settings.show(from: self?.view.window) }
            workspace.showWhatsNew = { [weak self] in self?.settings.openWhatsNew(from: self?.view.window) }
            child = workspace
        case .loading:
            child = NSViewController()
            child.view = NativeView(NativeStyle.blackout)
            let label = NativeStyle.label("Opening Caper…")
            label.frame = NSRect(x: 28, y: 28, width: 300, height: 30)
            child.view.addSubview(label)
        case .signedOut, .onboarding: child = SignInController(model: model)
        }
        addChild(child)
        child.view.frame = view.bounds
        child.view.autoresizingMask = [.width, .height]
        view.addSubview(child.view)
        // A review open while Caper finishes loading stays on top.
        if let review { view.addSubview(review) }
    }
}

/// While the update review is open, VoiceOver reaches only the review.
private final class RootView: NativeView {
    weak var modal: NSView?
    override func accessibilityChildren() -> [Any]? { modal.map { [$0] } ?? super.accessibilityChildren() }
}
