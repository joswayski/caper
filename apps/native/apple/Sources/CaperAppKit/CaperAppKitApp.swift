import AppKit
import CaperCore

@main @MainActor
final class CaperAppKitApp: NSObject, NSApplicationDelegate, NSMenuItemValidation {
    private var window: NSWindow?
    private var model: AppModel?
    private var settings: AppKitSettingsPresenter?

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
        // Never read, overwrite, or revoke the shipping app's session.
        let model = fixture ? CaperRuntime.makeModel() : AppModel(api: APIClient(
            tokenStore: KeychainTokenStore(service: "chat.caper.appkit-development.session|https://caper.chat")))
        self.model = model
        let settings = AppKitSettingsPresenter(model: model)
        self.settings = settings
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: fixture ? 1440 : 1180, height: fixture ? 900 : 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "Caper — AppKit development (not feature complete)"
        window.identifier = NSUserInterfaceItemIdentifier("caper-appkit-development")
        window.contentMinSize = NSSize(width: 390, height: 578)
        window.contentViewController = AppKitRootController(model: model, settings: settings)
        window.setContentSize(NSSize(width: fixture ? 1440 : 1180, height: fixture ? 900 : 760))
        window.center()
        self.window = window
        installMenus()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        Task { await model.start() }
    }

    func applicationDidBecomeActive(_ notification: Notification) { model?.applicationActivityChanged(active: true) }
    func applicationDidResignActive(_ notification: Notification) { model?.applicationActivityChanged(active: false) }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    @objc private func showSettings(_ sender: Any?) { settings?.show(from: window) }
    @objc private func showWhatsNew(_ sender: Any?) { settings?.openWhatsNew(from: window) }
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if menuItem.action == #selector(showWhatsNew(_:)) { return settings?.canOpenWhatsNew == true }
        return true
    }

    private func installMenus() {
        let menu = NSMenu()
        let app = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "Settings…", action: #selector(showSettings(_:)), keyEquivalent: ",").target = self
        appMenu.addItem(withTitle: "What’s new", action: #selector(showWhatsNew(_:)), keyEquivalent: "").target = self
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Quit Caper AppKit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
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
    private lazy var whatsNew = AppKitWhatsNew.presenter(model: model)
    private let observation = NativeObservation()
    private var shownPhase: AppModel.Phase?

    init(model: AppModel, settings: AppKitSettingsPresenter) {
        self.model = model; self.settings = settings; super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func loadView() { view = NativeView(NativeStyle.blackout) }
    override func viewDidLoad() {
        super.viewDidLoad()
        addChild(whatsNew)
        whatsNew.view.frame = .zero
        view.addSubview(whatsNew.view)
        observation.start(reading: { [model] in model.phase }) { [weak self] in self?.updatePhase($0) }
    }
    private func updatePhase(_ phase: AppModel.Phase) {
        guard shownPhase != phase else { return }
        shownPhase = phase
        for child in children where child !== whatsNew { child.view.removeFromSuperview(); child.removeFromParent() }
        let child: NSViewController
        switch phase {
        case .ready:
            let workspace = WorkspaceController(model: model)
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
    }
}
