#if os(macOS)
import AppKit
import ServiceManagement

/// One Settings window shares the shipping preferences and release history.
/// The development host passes no login item or updater, so it never registers
/// one or installs an update.
@MainActor public final class AppKitSettingsPresenter: NSObject, NSWindowDelegate {
    private let model: AppModel
    private let loginItem: CaperLoginItem?
    private let updater: AppUpdater?
    private let title: String
    private var window: NSWindow?
    private weak var previousWindow: NSWindow?
    private weak var previousResponder: NSResponder?
    private var previousContext: String?
    private var focusContext: String { "\(model.chat.editingContext):\(model.chat.threadRootID ?? "channel")" }

    public init(model: AppModel, loginItem: CaperLoginItem? = nil, updater: AppUpdater? = nil,
                title: String = "Caper Settings — AppKit development") {
        self.model = model; self.loginItem = loginItem; self.updater = updater; self.title = title
        super.init()
    }

    public var canOpenWhatsNew: Bool { !model.whatsNew.workspaceModalOpen || model.whatsNew.settingsOpen }

    public func show(from parent: NSWindow?) {
        // Do not transfer or acknowledge an already-presented main-window sheet.
        if model.whatsNew.presentation != nil && !model.whatsNew.settingsOpen {
            (parent?.attachedSheet ?? parent)?.makeKeyAndOrderFront(nil)
            return
        }
        if window?.isVisible != true {
            previousWindow = parent
            previousResponder = parent?.firstResponder
            previousContext = focusContext
        }
        if window == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 500, height: 380),
                styleMask: [.titled, .closable], backing: .buffered, defer: false)
            window.title = title
            window.identifier = AppKitWhatsNew.settingsWindowID
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: .darkAqua)
            window.delegate = self
            let loginItem = loginItem ?? CaperLoginItem(available: false, status: { .notRegistered }, register: {}, unregister: {})
            window.contentViewController = AppKitSettingsController(whatsNew: model.whatsNew, loginItem: loginItem, updater: updater)
            window.setContentSize(NSSize(width: 500, height: 380))
            window.center()
            self.window = window
        }
        // NSWindow.close() retains this controller for reuse. Its visibility
        // owns the release-note presentation gate.
        model.whatsNew.settingsOpen = true
        (window?.contentViewController as? AppKitSettingsController)?.refreshLoginItem()
        window?.makeKeyAndOrderFront(nil)
    }

    public func openWhatsNew(from parent: NSWindow?) {
        guard canOpenWhatsNew else { return }
        let owner = model.whatsNew.settingsOpen ? window : parent
        (owner?.attachedSheet ?? owner)?.makeKeyAndOrderFront(nil)
        guard model.whatsNew.presentation == nil else { return }
        model.whatsNew.open()
    }

    public func windowShouldClose(_ sender: NSWindow) -> Bool {
        // Escape/Done dismisses the nested sheet first, preserving its explicit
        // acknowledgment. Command-W must not reparent it onto the workspace.
        sender.attachedSheet == nil && model.whatsNew.presentation == nil
    }

    public func windowWillClose(_ notification: Notification) {
        let restore = window?.isKeyWindow == true
        model.whatsNew.settingsOpen = false
        let context = previousContext
        let responder = previousResponder as? NSView
        let parent = previousWindow
        previousResponder = nil; previousWindow = nil; previousContext = nil
        DispatchQueue.main.async { [weak self, weak responder, weak parent] in
            guard let self, restore, NSApp.isActive, let parent, parent.isVisible,
                  self.window?.isVisible != true, context == self.focusContext,
                  !self.model.whatsNew.workspaceModalOpen, parent.attachedSheet == nil,
                  NSApp.keyWindow == nil || NSApp.keyWindow === parent,
                  let responder, responder.window === parent, !responder.isHiddenOrHasHiddenAncestor else { return }
            parent.makeKeyAndOrderFront(nil)
            parent.makeFirstResponder(responder)
        }
    }
}

/// Startup, sounds, voice messages, updates and release notes.
@MainActor private final class AppKitSettingsController: NSViewController {
    private let whatsNew: WhatsNewModel
    private let loginItem: CaperLoginItem
    private let updater: AppUpdater?
    private let effects = CaperEffects.shared
    private let voiceMessages = VoiceMessageSettings.shared
    private let observation = AppKitObservation()
    private var timer: Timer?

    private lazy var launch = AppKitKitSwitchRow(title: "Launch at login", detail: "Open Caper when you sign in to your computer.",
                                                 identifier: "launch-at-login") { [weak self] on in self?.loginItem.setEnabled(on) }
    private let startupNotes = AppKitKit.stack([], spacing: 8)
    private lazy var sounds = AppKitKitSwitchRow(title: "Caper sound effects", detail: "Play sounds for messages and app interactions.",
                                                 identifier: "sound-effects") { [weak self] on in self?.effects.soundsEnabled = on }
    private lazy var sendOnStop = AppKitKitSwitchRow(title: VoiceMessage.sendOnStopTitle, detail: VoiceMessage.sendOnStopDetail,
                                                     identifier: "voice-messages-send-on-stop") { [weak self] on in self?.voiceMessages.sendOnStop = on }
    private let version = NSStackView()
    private let status = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let lastChecked = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let updateDetail = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private lazy var check = AppKitKitButton("Check for updates") { [weak self] in self?.controls.check?() }
    private lazy var notes = AppKitKitButton("What’s new") { [weak self] in self?.whatsNew.open() }

    init(whatsNew: WhatsNewModel, loginItem: CaperLoginItem, updater: AppUpdater?) {
        self.whatsNew = whatsNew; self.loginItem = loginItem; self.updater = updater
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private var controls: DesktopUpdateControls { updater?.controls ?? DesktopUpdateControls() }

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 500, height: 380))
        root.wantsLayer = true
        root.layer?.backgroundColor = AppKitKit.surface.cgColor
        check.setAccessibilityIdentifier("check-for-updates")
        notes.setAccessibilityIdentifier("open-whats-new")
        version.orientation = .horizontal; version.spacing = 4
        func section(_ title: String, _ rows: [NSView]) -> NSStackView {
            AppKitKit.stack([AppKitKit.label(title, size: 12, weight: .bold, color: AppKitKit.muted)] + rows, spacing: 14)
        }
        let updates = section("Updates", [version, AppKitKit.stack([status, lastChecked], spacing: 6), check, updateDetail])
        let content = AppKitKit.stack([
            AppKitKit.label("Settings", size: 20, weight: .bold),
            section("Startup", [launch, startupNotes]), AppKitKit.divider(),
            section("Sounds", [sounds]), AppKitKit.divider(),
            section("Voice messages", [sendOnStop]), AppKitKit.divider(),
            updates, notes,
        ], spacing: 20)
        for case let child as NSStackView in content.arrangedSubviews { AppKitKit.fillWidth(child, of: content) }
        for view in content.arrangedSubviews where !(view is NSStackView) && !(view is NSButton) && !(view is NSTextField) {
            AppKitKit.fillWidth(view, of: content)
        }
        for row in [launch, sounds, sendOnStop] as [NSView] {
            if let stack = row.superview as? NSStackView { AppKitKit.fillWidth(row, of: stack) }
        }
        let scroll = appKitKitScroll(content, insets: NSEdgeInsets(top: 24, left: 24, bottom: 24, right: 24))
        scroll.setAccessibilityIdentifier("desktop-settings")
        root.addSubview(scroll)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: root.leadingAnchor), scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: root.topAnchor), scroll.bottomAnchor.constraint(equalTo: root.bottomAnchor),
        ])
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        observation.start(reading: { [weak self] in self?.snapshot() }) { [weak self] _ in self?.render() }
        NotificationCenter.default.addObserver(self, selector: #selector(applicationBecameActive),
                                               name: NSApplication.didBecomeActiveNotification, object: nil)
    }

    override func viewWillAppear() {
        super.viewWillAppear()
        refreshLoginItem()
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 10, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.renderLastChecked() }
        }
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        timer?.invalidate(); timer = nil
    }

    func refreshLoginItem() { loginItem.refresh() }
    @objc private func applicationBecameActive() { loginItem.refresh() }

    /// Reads every observed value the window shows.
    private func snapshot() -> [AnyHashable] {
        let c = controls
        return [loginItem.registered, loginItem.available, loginItem.status == .requiresApproval, loginItem.error,
                effects.soundsEnabled, voiceMessages.sendOnStop,
                c.isChecking, c.review != nil, c.check != nil, c.lastSuccessfulCheck, c.installedVersion, c.sourceRevision]
    }

    private func render() {
        launch.isOn = loginItem.registered
        launch.isEnabled = loginItem.available
        startupNotes.arrangedSubviews.forEach { $0.removeFromSuperview() }
        if !loginItem.available {
            startupNotes.addArrangedSubview(AppKitKit.label("Startup registration is disabled in this build.", size: 12, color: AppKitKit.muted))
        }
        if loginItem.status == .requiresApproval {
            startupNotes.addArrangedSubview(AppKitKit.label("Launch at login needs approval in System Settings.", size: 12, color: AppKitKit.muted))
            startupNotes.addArrangedSubview(AppKitKitButton("Open Login Items Settings…") { SMAppService.openSystemSettingsLoginItems() })
        }
        if let error = loginItem.error {
            startupNotes.addArrangedSubview(AppKitKit.label(error, size: 12, color: AppKitKit.error))
            startupNotes.addArrangedSubview(AppKitKitButton("Dismiss startup error") { [weak self] in self?.loginItem.error = nil })
        }
        startupNotes.isHidden = startupNotes.arrangedSubviews.isEmpty
        sounds.isOn = effects.soundsEnabled
        sendOnStop.isOn = voiceMessages.sendOnStop

        let c = controls
        version.arrangedSubviews.forEach { $0.removeFromSuperview() }
        version.addArrangedSubview(AppKitKit.label(c.installedVersion, size: 12, color: AppKitKit.muted, wraps: false))
        if let revision = c.sourceRevision, let url = c.commitURL {
            version.addArrangedSubview(AppKitKit.label("·", size: 12, color: AppKitKit.muted, wraps: false))
            version.addArrangedSubview(AppKitKitLink(String(revision.prefix(7)), url: url, size: 12))
        }
        status.stringValue = c.isChecking ? "Checking for updates…" : c.review != nil ? "An update is available."
            : c.lastSuccessfulCheck != nil ? "Caper is up to date." : "Not checked yet."
        check.isEnabled = c.check != nil && !c.isChecking
        updateDetail.stringValue = c.check == nil ? "Update checks are available in packaged release builds." : "Checks automatically every minute."
        renderLastChecked()
    }

    private func renderLastChecked() {
        guard let date = controls.lastSuccessfulCheck else { lastChecked.stringValue = "Last checked: Never"; return }
        let now = Date()
        if now.timeIntervalSince(date) < 10 { lastChecked.stringValue = "Last checked: a few seconds ago"; return }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        lastChecked.stringValue = "Last checked: \(formatter.localizedString(for: date, relativeTo: now))"
    }
}

/// Release notes for the AppKit app: a zero-size controller in the main window
/// presents What's new as a sheet on the main window, or on Settings while
/// Settings is open, and runs the loading and automatic-presentation loops.
@MainActor public enum AppKitWhatsNew {
    static let settingsWindowID = NSUserInterfaceItemIdentifier("caper-appkit-settings")

    public static func presenter(model: AppModel) -> NSViewController {
        AppKitWhatsNewController(model: model)
    }
}

@MainActor private final class AppKitWhatsNewController: NSViewController {
    private let model: AppModel
    private let observation = AppKitObservation()
    private let readiness = AppKitObservation()
    private var sheet: AppKitWhatsNewSheet?
    private weak var owner: NSWindow?
    private var timer: Timer?
    private var refreshing = false
    private var lastRefresh: Date?
    private var wasLive = false

    init(model: AppModel) { self.model = model; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func loadView() { view = NSView(frame: .zero) }

    private var ready: Bool { model.phase == .ready && model.account != nil && model.spacesLoaded && !model.restoringWorkspace }

    override func viewDidLoad() {
        super.viewDidLoad()
        observation.start(reading: { [model] in (model.whatsNew.presentation?.id, model.whatsNew.settingsOpen) }) { [weak self] _ in
            self?.sync()
        }
        var wasReady = false
        readiness.start(reading: { [model] in model.phase == .ready }) { [model] isReady in
            if isReady && !wasReady { CaperRuntime.showWhatsNewPreview(model) }
            wasReady = isReady
        }
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        sync()
    }

    /// SwiftUI's two `.task`s: load the snapshot (retrying each minute until
    /// one arrives) and present unread highlights when nothing else is modal.
    private func tick() {
        let live = ready && NSApp.isActive
        defer { wasLive = live }
        guard live else { return }
        if !wasLive { lastRefresh = nil }
        let notes = model.whatsNew
        if !notes.hasSnapshot, !refreshing, lastRefresh.map({ Date().timeIntervalSince($0) >= 60 }) ?? true {
            refreshing = true; lastRefresh = Date()
            Task { [weak self] in
                await notes.refresh()
                self?.refreshing = false
            }
        }
        guard model.voice.phase == .idle || model.voice.phase == .failed else { return }
        if !notes.workspaceModalOpen && !notes.settingsOpen && !WhatsNewModel.nativeModalOpen {
            notes.presentAutomatically()
        }
    }

    private func sync() {
        let notes = model.whatsNew
        guard let display = notes.presentation else { endSheet(); return }
        let target = notes.settingsOpen ? NSApp.windows.first { $0.identifier == AppKitWhatsNew.settingsWindowID } : view.window
        if let sheet, sheet.display.id == display.id, owner === target { return }
        endSheet()
        guard let target, target.isVisible else { return }
        let next = AppKitWhatsNewSheet(notes: notes, display: display) { [weak self] in self?.endSheet() }
        sheet = next; owner = target
        target.beginSheet(next) { _ in }
    }

    private func endSheet() {
        guard let sheet else { return }
        self.sheet = nil
        let display = sheet.display
        owner?.endSheet(sheet)
        owner = nil
        sheet.orderOut(nil)
        model.whatsNew.dismiss(display)
    }
}

/// The release highlights sheet: the face, title and installed version, then
/// each day's releases, and Done (or Got it for an automatic presentation).
@MainActor private final class AppKitWhatsNewSheet: NSWindow {
    private let notes: WhatsNewModel
    private(set) var display: WhatsNewDisplay
    private let finish: () -> Void
    private let observation = AppKitObservation()
    private let body = AppKitKit.stack([], spacing: 16)

    init(notes: WhatsNewModel, display: WhatsNewDisplay, close: @escaping () -> Void) {
        self.notes = notes; self.display = display; self.finish = close
        super.init(contentRect: NSRect(x: 0, y: 0, width: 520, height: 540), styleMask: [.titled], backing: .buffered, defer: false)
        appearance = NSAppearance(named: .darkAqua)
        isReleasedWhenClosed = false
        contentView = makeContent()
        observation.start(reading: { [notes] in (notes.presentation?.id, notes.presentation?.releases?.count, notes.loading, notes.error) }) { [weak self] _ in
            self?.render()
        }
    }

    private var installedVersion: String {
        (Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String)
            .map { "Installed version \($0)" } ?? "Development build"
    }

    private func makeContent() -> NSView {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 520, height: 540))
        root.wantsLayer = true
        root.layer?.backgroundColor = AppKitKit.surface.cgColor
        root.setAccessibilityElement(true)
        root.setAccessibilityRole(.group)
        root.setAccessibilityIdentifier("whats-new")

        let title = AppKitKit.label("What’s new", size: 20, weight: .bold, wraps: false)
        title.setAccessibilityRole(.staticText)
        var heading: [NSView] = [AppKitKit.image("caper-face", size: 36), title]
        if WhatsNewSnapshot.validRevision(notes.revision),
           let url = URL(string: "https://github.com/joswayski/caper/commit/\(notes.revision)") {
            heading.append(AppKitKitLink(installedVersion, url: url))
        } else {
            heading.append(AppKitKit.label(installedVersion, size: 13, color: AppKitKit.muted, wraps: false))
        }
        let headingStack = AppKitKit.stack(heading, spacing: 2)
        headingStack.setCustomSpacing(10, after: heading[0])
        let closeButton = AppKitKitButton("Close") { [weak self] in self?.finish() }
        closeButton.setAccessibilityLabel("Close What’s new")
        closeButton.keyEquivalent = "\u{1b}"
        let header = NSView(); header.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(headingStack); header.addSubview(closeButton)

        let scroll = appKitKitScroll(body, insets: NSEdgeInsets(top: 22, left: 22, bottom: 22, right: 22))
        let done = AppKitKitButton(display.automatic ? "Got it" : "Done", kind: .primary) { [weak self] in self?.finish() }
        done.keyEquivalent = "\r"
        let divider = AppKitKit.divider()
        for view in [header, divider, scroll, done] as [NSView] { root.addSubview(view) }
        NSLayoutConstraint.activate([
            header.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 22),
            header.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -22),
            header.topAnchor.constraint(equalTo: root.topAnchor, constant: 22),
            headingStack.leadingAnchor.constraint(equalTo: header.leadingAnchor),
            headingStack.topAnchor.constraint(equalTo: header.topAnchor),
            headingStack.bottomAnchor.constraint(equalTo: header.bottomAnchor),
            closeButton.trailingAnchor.constraint(equalTo: header.trailingAnchor),
            closeButton.topAnchor.constraint(equalTo: header.topAnchor),
            divider.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            divider.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 22),
            scroll.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: divider.bottomAnchor),
            done.topAnchor.constraint(equalTo: scroll.bottomAnchor, constant: 22),
            done.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -22),
            done.widthAnchor.constraint(greaterThanOrEqualToConstant: 120),
            done.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -22),
        ])
        return root
    }

    private func render() {
        if display.releases == nil, let loaded = notes.presentation, loaded.id == display.id { display = loaded }
        body.arrangedSubviews.forEach { $0.removeFromSuperview() }
        func add(_ view: NSView, fill: Bool = true) {
            body.addArrangedSubview(view)
            if fill { AppKitKit.fillWidth(view, of: body) }
        }
        if let releases = display.releases {
            if releases.isEmpty { add(AppKitKit.label("No release highlights for this version yet.", color: AppKitKit.muted)) }
            let days = Dictionary(grouping: releases) { String($0.publishedAt.prefix(10)) }
            for day in days.keys.sorted(by: >) {
                add(AppKitKit.label(releaseDayLabel(day), size: 12, weight: .bold, color: AppKitKit.muted))
                for release in days[day] ?? [] {
                    var rows: [NSView] = []
                    if let url = URL(string: "https://github.com/joswayski/caper/commit/\(release.revision)") {
                        rows.append(AppKitKitLink(release.version.map { "Version \($0)" } ?? String(release.revision.prefix(7)), url: url, size: 12))
                    }
                    for highlight in release.highlights {
                        let title = AppKitKit.label(highlight.title, size: 16, weight: .bold)
                        let text = AppKitKit.label(highlight.body, size: 14, color: AppKitKit.muted)
                        let item = AppKitKit.stack([title, text], spacing: 4)
                        AppKitKit.fillWidth(title, of: item); AppKitKit.fillWidth(text, of: item)
                        rows.append(item)
                    }
                    let release = AppKitKit.stack(rows, spacing: 12)
                    for row in rows where row is NSStackView { AppKitKit.fillWidth(row, of: release) }
                    add(release)
                    add(AppKitKit.divider())
                }
            }
        } else if notes.loading {
            let spinner = NSProgressIndicator()
            spinner.style = .spinning; spinner.controlSize = .small; spinner.startAnimation(nil)
            spinner.setAccessibilityLabel("Loading release highlights")
            add(spinner, fill: false)
            add(AppKitKit.label("Loading release highlights…", color: AppKitKit.muted))
        } else if let error = notes.error {
            add(AppKitKit.label(error, color: AppKitKit.error))
            add(AppKitKitButton("Retry") { [notes] in Task { await notes.refresh() } }, fill: false)
        }
    }

    override func cancelOperation(_ sender: Any?) { finish() }
}
#endif
