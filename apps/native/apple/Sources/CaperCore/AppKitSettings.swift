#if os(macOS)
import AppKit
import Observation
import ServiceManagement

/// "Keep Caper running when the window closes": a per-device setting, on by
/// default. On, closing the window hides it as Mac apps do, so a call and
/// notifications carry on and the Dock icon brings it back; off, closing it
/// quits. Windows and Linux keep the same setting (caper_client::background).
@MainActor @Observable public final class DesktopWindowSettings {
    public static let shared = DesktopWindowSettings()
    static let key = "keep-running-when-closed-v1"
    public static let title = "Keep Caper running when the window closes"
    static let detail = "Closing the window hides Caper, so calls and notifications keep going; click Caper in the Dock to bring it back. Off: closing the window quits Caper."
    @ObservationIgnored private let defaults: UserDefaults
    public var keepRunning: Bool {
        didSet { defaults.set(keepRunning, forKey: Self.key) }
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        keepRunning = defaults.object(forKey: Self.key) as? Bool ?? true
    }
}

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
        let parityMode = ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity"
        let useSheet = !parityMode && parent != nil && parent?.attachedSheet == nil
        if window == nil {
            let window = AppKitSettingsSheetWindow(
                contentRect: NSRect(x: 0, y: 0, width: 540, height: 600),
                styleMask: useSheet ? [.titled] : [.titled, .closable],
                backing: .buffered,
                defer: false
            )
            window.title = title
            window.identifier = AppKitWhatsNew.settingsWindowID
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: CaperThemeStore.shared.active.isLight ? .aqua : .darkAqua)
            window.backgroundColor = CaperThemeStore.baseColor
            window.delegate = self
            window.onCancel = { [weak self] in self?.cancel() }
            let loginItem = loginItem ?? CaperLoginItem(available: false, status: { .notRegistered }, register: {}, unregister: {})
            window.contentViewController = AppKitSettingsController(
                model: model,
                whatsNew: model.whatsNew,
                loginItem: loginItem,
                updater: updater,
                onClose: { [weak self] in self?.dismiss() }
            )
            window.setContentSize(NSSize(width: 540, height: 600))
            window.center()
            self.window = window
        }
        // NSWindow.close() retains this controller for reuse. Its visibility
        // owns the release-note presentation gate.
        model.whatsNew.settingsOpen = true
        (window?.contentViewController as? AppKitSettingsController)?.refreshLoginItem()
        guard let window else { return }
        if useSheet, let parent {
            if window.sheetParent == nil {
                parent.beginSheet(window) { [weak self] _ in
                    self?.handleDismissed()
                }
            }
        } else {
            window.makeKeyAndOrderFront(nil)
        }
    }

    public func dismiss() {
        guard let window else { return }
        guard windowShouldClose(window) else { return }
        if let parent = window.sheetParent {
            parent.endSheet(window)
            window.orderOut(nil)
        } else {
            window.close()
        }
    }

    /// Escape leaves an open sub-page (Notifications, Privacy, Edit profile,
    /// Audio test) for Settings, then closes Settings.
    private func cancel() {
        if (window?.contentViewController as? AppKitSettingsController)?.cancelPage() == true { return }
        dismiss()
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
        handleDismissed()
    }

    private func handleDismissed() {
        let restore = window?.isKeyWindow == true || window?.sheetParent != nil
        model.whatsNew.settingsOpen = false
        (window?.contentViewController as? AppKitSettingsController)?.closePage()
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

@MainActor private final class AppKitSettingsSheetWindow: NSWindow {
    var onCancel: (() -> Void)?
    override func cancelOperation(_ sender: Any?) { onCancel?() }
}

/// Clickable miniature app preview card for choosing Obsidian (Dark) or Paper (Light).
@MainActor private final class AppKitThemePresetCard: NSView {
    private let preset: CaperThemeDocument
    private let onSelect: () -> Void
    private var hovered = false
    private var pressed = false
    private var tracking: NSTrackingArea?

    var isSelected = false {
        didSet {
            if oldValue != isSelected {
                setAccessibilitySelected(isSelected)
                setAccessibilityValue(isSelected ? 1 : 0)
                needsDisplay = true
            }
        }
    }

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { KeyboardTabOrder.keyboardFocusAllowed }
    override var canBecomeKeyView: Bool { acceptsFirstResponder && !isHiddenOrHasHiddenAncestor && window != nil }

    init(preset: CaperThemeDocument, onSelect: @escaping () -> Void) {
        self.preset = preset
        self.onSelect = onSelect
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        setAccessibilityElement(true)
        setAccessibilityRole(.radioButton)
        setAccessibilityLabel("\(preset.name) (\(preset.isLight ? "Light" : "Dark"))")
        setAccessibilityValue(0)
        setAccessibilityIdentifier("appearance-theme-\(preset.id)")
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 102),
        ])
        NotificationCenter.default.addObserver(self, selector: #selector(handleThemeChange),
                                               name: CaperThemeStore.didChangeNotification, object: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    @objc private func handleThemeChange() { needsDisplay = true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { hovered = true; needsDisplay = true }
    override func mouseExited(with event: NSEvent) { hovered = false; pressed = false; needsDisplay = true }
    override func mouseDown(with event: NSEvent) { pressed = true; needsDisplay = true }
    override func mouseUp(with event: NSEvent) {
        let inside = bounds.contains(convert(event.locationInWindow, from: nil))
        pressed = false
        needsDisplay = true
        if inside { onSelect() }
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 48 {
            KeyboardTabOrder.step(in: window, forward: !event.modifierFlags.contains(.shift), from: self)
            return
        }
        if [36, 49, 76].contains(event.keyCode) {
            onSelect()
            return
        }
        super.keyDown(with: event)
    }

    override func becomeFirstResponder() -> Bool {
        let ok = super.becomeFirstResponder()
        needsDisplay = true
        return ok
    }

    override func resignFirstResponder() -> Bool {
        let ok = super.resignFirstResponder()
        needsDisplay = true
        return ok
    }

    override func accessibilityPerformPress() -> Bool {
        onSelect()
        return true
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        let isLight = CaperThemeStore.shared.active.isLight
        let focused = window?.firstResponder === self && window?.isKeyWindow == true
        let cardRect = bounds.insetBy(dx: 2, dy: 2)
        let cardPath = NSBezierPath(roundedRect: cardRect, xRadius: 13, yRadius: 13)

        var fill = pressed ? AppKitKit.composer : AppKitKit.raised
        if hovered && !pressed {
            fill = fill.blended(withFraction: isLight ? 0.04 : 0.08, of: isLight ? .black : .white) ?? fill
        }
        fill.setFill()
        cardPath.fill()

        if isSelected {
            let haloRect = bounds.insetBy(dx: 0.75, dy: 0.75)
            let haloPath = NSBezierPath(roundedRect: haloRect, xRadius: 14, yRadius: 14)
            AppKitKit.terracottaBright.withAlphaComponent(0.26).setStroke()
            haloPath.lineWidth = 3
            haloPath.stroke()
        }

        let borderColor: NSColor = (isSelected || focused)
            ? AppKitKit.terracottaBright
            : hovered
                ? AppKitKit.border.blended(withFraction: isLight ? 0.16 : 0.22, of: isLight ? .black : .white) ?? AppKitKit.border
                : AppKitKit.border
        borderColor.setStroke()
        cardPath.lineWidth = (isSelected || focused) ? 1.5 : 1
        cardPath.stroke()

        // Miniature Caper preview swatch
        let swatchRect = NSRect(x: cardRect.minX + 10, y: cardRect.minY + 10, width: cardRect.width - 20, height: 54)
        let frameColor = CaperThemeStore.nsColor(fromHex: preset.surfaces.frame) ?? .black
        let sidebarColor = CaperThemeStore.nsColor(fromHex: preset.surfaces.sidebar) ?? frameColor
        let convoColor = CaperThemeStore.nsColor(fromHex: preset.surfaces.conversation) ?? frameColor
        let brandColor = CaperThemeStore.nsColor(fromHex: preset.brand.caper) ?? AppKitKit.terracotta
        let primaryText = CaperThemeStore.nsColor(fromHex: preset.text.primary) ?? .white
        let mutedText = CaperThemeStore.nsColor(fromHex: preset.text.muted) ?? .gray
        let actionColor = CaperThemeStore.nsColor(fromHex: preset.action.primary) ?? primaryText

        let swatchPath = NSBezierPath(roundedRect: swatchRect, xRadius: 9, yRadius: 9)
        frameColor.setFill()
        swatchPath.fill()
        NSColor(srgbRed: 0.5, green: 0.5, blue: 0.5, alpha: 0.24).setStroke()
        swatchPath.lineWidth = 1
        swatchPath.stroke()

        let inner = swatchRect.insetBy(dx: 6, dy: 6)
        let sidebarW = floor(inner.width * 0.28)
        let sidebarRect = NSRect(x: inner.minX, y: inner.minY, width: sidebarW, height: inner.height)
        let convoRect = NSRect(x: sidebarRect.maxX + 5, y: inner.minY, width: inner.width - sidebarW - 5, height: inner.height)

        sidebarColor.setFill()
        NSBezierPath(roundedRect: sidebarRect, xRadius: 5, yRadius: 5).fill()
        convoColor.setFill()
        NSBezierPath(roundedRect: convoRect, xRadius: 6, yRadius: 6).fill()

        func drawBar(_ rect: NSRect, color: NSColor, alpha: CGFloat = 0.72) {
            color.withAlphaComponent(alpha).setFill()
            NSBezierPath(roundedRect: rect, xRadius: rect.height / 2, yRadius: rect.height / 2).fill()
        }

        let sbPad: CGFloat = 5
        let sbW = max(6, sidebarRect.width - sbPad * 2)
        drawBar(NSRect(x: sidebarRect.minX + sbPad, y: sidebarRect.minY + 6, width: sbW * 0.78, height: 4.5), color: brandColor, alpha: 0.9)
        drawBar(NSRect(x: sidebarRect.minX + sbPad, y: sidebarRect.minY + 15, width: sbW * 0.56, height: 4.5), color: mutedText)
        drawBar(NSRect(x: sidebarRect.minX + sbPad, y: sidebarRect.minY + 24, width: sbW * 0.66, height: 4.5), color: mutedText)

        let cvPad: CGFloat = 6
        let cvW = max(12, convoRect.width - cvPad * 2)
        drawBar(NSRect(x: convoRect.minX + cvPad, y: convoRect.minY + 6, width: cvW * 0.68, height: 4.5), color: primaryText, alpha: 0.78)
        drawBar(NSRect(x: convoRect.minX + cvPad, y: convoRect.minY + 14.5, width: cvW * 0.48, height: 4.5), color: mutedText, alpha: 0.68)
        let pillRect = NSRect(x: convoRect.maxX - cvPad - 26, y: convoRect.maxY - cvPad - 8.5, width: 26, height: 8.5)
        drawBar(pillRect, color: actionColor, alpha: 1)

        // Footer label row
        let titleAttrs: [NSAttributedString.Key: Any] = [
            .font: AppKitKit.font(13, weight: .bold),
            .foregroundColor: AppKitKit.text,
        ]
        let tagAttrs: [NSAttributedString.Key: Any] = [
            .font: AppKitKit.font(11.5, weight: .medium),
            .foregroundColor: AppKitKit.muted,
        ]
        let titleStr = NSAttributedString(string: preset.name, attributes: titleAttrs)
        let tagStr = NSAttributedString(string: preset.isLight ? "Light" : "Dark", attributes: tagAttrs)
        let metaY = swatchRect.maxY + 9
        titleStr.draw(at: NSPoint(x: cardRect.minX + 12, y: metaY))
        let tagSize = tagStr.size()
        tagStr.draw(at: NSPoint(x: cardRect.maxX - 12 - tagSize.width, y: metaY + 1))
    }
}

/// Every client's Settings sections in order: Profile, Appearance,
/// Notifications, Voice & audio, Startup, Privacy & blocked accounts, About
/// and Log out. Edit profile, Notifications, Privacy and the audio dialogs
/// open as pages in this window; their close (or Escape) returns here.
@MainActor private final class AppKitSettingsController: NSViewController {
    private let model: AppModel
    private let whatsNew: WhatsNewModel
    private let loginItem: CaperLoginItem
    private let updater: AppUpdater?
    private let onClose: () -> Void
    private let voiceMessages = VoiceMessageSettings.shared
    private let closing = DesktopWindowSettings.shared
    private let themeStore = CaperThemeStore.shared
    private let observation = AppKitObservation()

    private lazy var closeButton = AppKitKitButton("Done", kind: .secondary) { [weak self] in self?.onClose() }
    private lazy var obsidianCard = AppKitThemePresetCard(preset: .obsidian) { [weak self] in
        self?.themeStore.selectPreset("obsidian")
    }
    private lazy var paperCard = AppKitThemePresetCard(preset: .paper) { [weak self] in
        self?.themeStore.selectPreset("paper")
    }

    private lazy var launch = AppKitKitSwitchRow(title: "Launch at login", detail: "Open Caper when you sign in to your computer.",
                                                 identifier: "launch-at-login") { [weak self] on in self?.loginItem.setEnabled(on) }
    private let startupNotes = AppKitKit.stack([], spacing: 8)
    private lazy var keepRunning = AppKitKitSwitchRow(title: DesktopWindowSettings.title, detail: DesktopWindowSettings.detail,
                                                      identifier: "keep-running-when-closed") { [weak self] on in self?.closing.keepRunning = on }
    private lazy var sendOnStop = AppKitKitSwitchRow(title: VoiceMessage.sendOnStopTitle, detail: VoiceMessage.sendOnStopDetail,
                                                     identifier: "voice-messages-send-on-stop") { [weak self] on in self?.voiceMessages.sendOnStop = on }
    private let version = NSStackView()
    private let status = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private lazy var check = AppKitKitButton("Check for updates") { [weak self] in self?.controls.check?() }
    private lazy var notes = AppKitKitButton("What’s new") { [weak self] in self?.whatsNew.open() }

    private let avatar = AppKitAvatar()
    private let profileName = AppKitKit.label("", size: 15, weight: .bold, wraps: false)
    private let profileUsername = AppKitKit.label("", size: 12, color: AppKitKit.muted, wraps: false)
    private let pauseStatus = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private lazy var editProfile = AppKitKitButton("Edit profile") { [weak self] in self?.openProfile() }
    private lazy var notificationSettings = AppKitKitButton(NotificationLabels.notifications) { [weak self] in
        guard let self else { return }
        self.showPage(AppKitNotificationsDialog(model: self.model, close: { [weak self] in self?.closePage() }))
    }
    private lazy var audioTest = AppKitKitButton("Audio test") { [weak self] in
        guard let self else { return }
        self.showPage(AppKitAudioTest(voice: self.model.voice, debugEnabled: self.model.account?.debugEnabled == true,
                                      close: { [weak self] in self?.closePage() }))
    }
    private lazy var connection = AppKitKitButton("Connection details") { [weak self] in
        guard let self else { return }
        self.showPage(AppKitAudioDiagnostics(voice: self.model.voice, kind: .connection, close: { [weak self] in self?.closePage() }))
    }
    private lazy var diagnostics = AppKitKitButton("Audio diagnostics") { [weak self] in
        guard let self, self.model.account?.debugEnabled == true else { return }
        self.showPage(AppKitAudioDiagnostics(voice: self.model.voice, kind: .processing, close: { [weak self] in self?.closePage() }))
    }
    private lazy var privacy = AppKitKitButton("Privacy and blocked accounts") { [weak self] in
        guard let self else { return }
        self.showPage(AppKitPrivacyDialog(model: self.model, close: { [weak self] in self?.closePage() }))
    }
    private lazy var logOut = AppKitKitButton("Log out", kind: .destructive) { [weak self] in
        guard let self else { return }
        let model = self.model
        self.onClose()
        Task { await model.logout() }
    }
    /// Profile, Notifications, Privacy and Log out, shown while signed in.
    private var accountViews: [NSView] = []
    private var scroll: NSScrollView!
    private var page: NSView?
    private var profile: AppKitProfilePresentation?

    init(model: AppModel, whatsNew: WhatsNewModel, loginItem: CaperLoginItem, updater: AppUpdater?, onClose: @escaping () -> Void) {
        self.model = model; self.whatsNew = whatsNew; self.loginItem = loginItem; self.updater = updater; self.onClose = onClose
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private var controls: DesktopUpdateControls { updater?.controls ?? DesktopUpdateControls() }

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 540, height: 600))
        root.wantsLayer = true
        root.layer?.backgroundColor = AppKitKit.surface.cgColor
        check.setAccessibilityIdentifier("check-for-updates")
        notes.setAccessibilityIdentifier("open-whats-new")
        closeButton.setAccessibilityIdentifier("close-settings")
        version.orientation = .horizontal; version.spacing = 4
        // Built once, so its "Copied" feedback and keyboard focus survive renders.
        version.addArrangedSubview(appKitVersionControl())
        func section(_ title: String, _ rows: [NSView]) -> NSStackView {
            AppKitKit.stack([AppKitKit.label(title, size: 12, weight: .bold, color: AppKitKit.muted)] + rows, spacing: 12)
        }

        let header = NSStackView(views: [AppKitKit.label("Settings", size: 20, weight: .bold, wraps: false), NSView(), closeButton])
        header.orientation = .horizontal; header.alignment = .centerY; header.translatesAutoresizingMaskIntoConstraints = false

        let appearanceNote = AppKitKit.label("Choose between Obsidian Dark and Paper Light.", size: 12, color: AppKitKit.muted)
        let themeCards = NSStackView(views: [obsidianCard, paperCard])
        themeCards.orientation = .horizontal
        themeCards.distribution = .fillEqually
        themeCards.spacing = 10
        themeCards.translatesAutoresizingMaskIntoConstraints = false
        let appearance = section("Appearance", [appearanceNote, themeCards])

        func row(_ views: [NSView]) -> NSStackView {
            let row = NSStackView(views: views); row.orientation = .horizontal; row.spacing = 8; return row
        }
        avatar.translatesAutoresizingMaskIntoConstraints = false
        avatar.widthAnchor.constraint(equalToConstant: 40).isActive = true
        avatar.heightAnchor.constraint(equalToConstant: 40).isActive = true
        let names = AppKitKit.stack([profileName, profileUsername], spacing: 2)
        let identity = row([avatar, names]); identity.spacing = 12; identity.alignment = .centerY
        identity.setAccessibilityElement(true); identity.setAccessibilityRole(.group); identity.setAccessibilityIdentifier("settings-profile")
        editProfile.setAccessibilityIdentifier("settings-edit-profile")
        notificationSettings.setAccessibilityIdentifier("open-notifications")
        pauseStatus.setAccessibilityIdentifier("settings-do-not-disturb-status")
        privacy.setAccessibilityIdentifier("open-privacy")
        logOut.setAccessibilityIdentifier("log-out")
        let profile = section("Profile", [identity, editProfile])
        let notifications = section(NotificationLabels.notifications, [pauseStatus, notificationSettings])
        let voice = section("Voice & audio", [sendOnStop, row([audioTest, connection, diagnostics])])
        let privacySection = section("Privacy & blocked accounts", [privacy])
        let buttons = row([check, notes])
        let about = section("About", [version, buttons, status])
        let dividers = (0..<7).map { _ in AppKitKit.divider() }
        accountViews = [profile, dividers[0], notifications, dividers[2], privacySection, dividers[5], dividers[6], logOut]
        let content = AppKitKit.stack([
            header,
            profile, dividers[0],
            appearance, dividers[1],
            notifications, dividers[2],
            voice, dividers[3],
            // Desktop-only: after Voice & audio, as on Windows and Linux.
            section("Startup", [launch, startupNotes, keepRunning]), dividers[4],
            privacySection, dividers[5],
            about, dividers[6],
            logOut,
        ], spacing: 18)
        for case let child as NSStackView in content.arrangedSubviews { AppKitKit.fillWidth(child, of: content) }
        for view in content.arrangedSubviews where !(view is NSStackView) && !(view is NSButton) && !(view is NSTextField) {
            AppKitKit.fillWidth(view, of: content)
        }
        for row in [themeCards, launch, keepRunning, sendOnStop] as [NSView] {
            if let stack = row.superview as? NSStackView { AppKitKit.fillWidth(row, of: stack) }
        }
        let scroll = appKitKitScroll(content, insets: NSEdgeInsets(top: 22, left: 24, bottom: 24, right: 24))
        self.scroll = scroll
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
        NotificationCenter.default.addObserver(self, selector: #selector(handleThemeChange),
                                               name: CaperThemeStore.didChangeNotification, object: nil)
    }

    override func viewWillAppear() {
        super.viewWillAppear()
        refreshLoginItem()
        render()
        if model.account != nil { Task { [model] in await model.loadNotificationSettings() } }
    }

    // MARK: Pages

    private func showPage(_ next: NSView) {
        closePage()
        page = next
        next.translatesAutoresizingMaskIntoConstraints = false
        scroll.isHidden = true
        view.addSubview(next)
        NSLayoutConstraint.activate([
            next.leadingAnchor.constraint(equalTo: view.leadingAnchor), next.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            next.topAnchor.constraint(equalTo: view.topAnchor), next.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        if let focus = (next as? AppKitDialogContent)?.initialFocus { view.window?.makeFirstResponder(focus) }
    }

    private func openProfile() {
        let profile = AppKitProfilePresentation(model: model, close: { [weak self] in self?.closePage() })
        addChild(profile)
        showPage(profile.view)
        self.profile = profile
    }

    /// Back to Settings from a page; nothing when none is open.
    func closePage() {
        guard let page else { return }
        self.page = nil
        if let profile { profile.stop(); profile.removeFromParent(); self.profile = nil }
        page.removeFromSuperview()
        scroll.isHidden = false
    }

    /// Escape: true when a page was open. The profile editor asks before
    /// discarding changes; a page with a request running stays open.
    func cancelPage() -> Bool {
        guard let page else { return false }
        if let profile { profile.requestClose() }
        else if (page as? AppKitDialogContent)?.dismissDisabled != true { closePage() }
        return true
    }

    func refreshLoginItem() { loginItem.refresh() }
    @objc private func applicationBecameActive() { loginItem.refresh() }
    @objc private func handleThemeChange() {
        view.layer?.backgroundColor = AppKitKit.surface.cgColor
        view.window?.backgroundColor = AppKitKit.surface
        view.window?.appearance = NSAppearance(named: themeStore.active.isLight ? .aqua : .darkAqua)
        obsidianCard.isSelected = themeStore.active.id != "paper"
        paperCard.isSelected = themeStore.active.id == "paper"
        view.needsDisplay = true
    }

    /// Reads every observed value the window shows.
    private func snapshot() -> [AnyHashable] {
        let c = controls
        return [loginItem.registered, loginItem.available, loginItem.status == .requiresApproval, loginItem.error,
                voiceMessages.sendOnStop, closing.keepRunning, themeStore.active.id,
                model.account?.id, model.account?.displayName, model.account?.username, model.account?.avatarId,
                model.account?.avatarAsset, model.account?.debugEnabled, model.doNotDisturb,
                model.voice.phase == .connected, model.voice.phase == .leaving,
                c.isChecking, c.review != nil, c.check != nil, c.lastSuccessfulCheck, c.installedVersion, c.sourceRevision]
    }

    private func render() {
        view.layer?.backgroundColor = AppKitKit.surface.cgColor
        obsidianCard.isSelected = themeStore.active.id != "paper"
        paperCard.isSelected = themeStore.active.id == "paper"
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
        sendOnStop.isOn = voiceMessages.sendOnStop
        keepRunning.isOn = closing.keepRunning
        let account = model.account
        for view in accountViews where view.isHidden != (account == nil) { view.isHidden = account == nil }
        if let account {
            avatar.update(account)
            profileName.stringValue = account.displayName ?? account.username ?? ""
            profileUsername.stringValue = account.username.map { "@\($0)" } ?? ""
        }
        pauseStatus.stringValue = "\(DoNotDisturbLabels.title): \(DoNotDisturbLabels.status(model.doNotDisturb))"
        audioTest.isEnabled = model.voice.phase != .leaving
        connection.isHidden = model.voice.phase != .connected
        diagnostics.isHidden = account?.debugEnabled != true

        let c = controls
        status.stringValue = c.isChecking ? "Checking for updates…" : c.review != nil ? "An update is available."
            : c.lastSuccessfulCheck != nil ? "Caper is up to date." : ""
        status.isHidden = status.stringValue.isEmpty
        check.isEnabled = c.check != nil && !c.isChecking
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
    private let voiceSpeaking = AppKitObservation()
    private lazy var speaking = NativeVoiceSpeaking(voice: model.voice)
    private var sheet: AppKitWhatsNewSheet?
    private weak var owner: NSWindow?
    private var dismissMonitor: Any?
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
        // The call's speaking rings; nothing samples while no Caper window
        // shows (minimized, covered or on another Space).
        voiceSpeaking.start(reading: { [speaking] in speaking.sync() }, apply: { _ in })
        NotificationCenter.default.addObserver(self, selector: #selector(occlusionChanged),
            name: NSApplication.didChangeOcclusionStateNotification, object: nil)
        occlusionChanged()
        var wasReady = false
        readiness.start(reading: { [model] in model.phase == .ready }) { [model] isReady in
            if isReady && !wasReady { CaperRuntime.showWhatsNewPreview(model) }
            wasReady = isReady
        }
        // tick() does nothing while another app is active, so neither does the
        // timer, and it stops once the notes are loaded and read.
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(activityChanged), name: NSApplication.didBecomeActiveNotification, object: nil)
        center.addObserver(self, selector: #selector(activityChanged), name: NSApplication.didResignActiveNotification, object: nil)
        activityChanged()
    }

    @objc private func occlusionChanged() { speaking.sampling = NSApp.occlusionState.contains(.visible) }

    @objc private func activityChanged() {
        if NSApp.isActive, timer == nil {
            timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.tick() }
            }
        } else if !NSApp.isActive {
            timer?.invalidate(); timer = nil
            tick()
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
        // Until Caper is next activated: no more ticks (two a second) once
        // there is nothing to load or present.
        if notes.settled { timer?.invalidate(); timer = nil; return }
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
        next.fitHeight()
        // A click on the window behind (or Settings' own parent) closes it, like
        // web's light-dismiss dialog, and goes no further. Escape closes it even
        // while selected text has focus, which would otherwise take the key.
        dismissMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .keyDown]) { [weak self, weak next] event in
            guard let self, let next, let window = event.window, let parent = next.sheetParent else { return event }
            let close = event.type == .keyDown
                ? window === next && event.keyCode == 53
                : sequence(first: parent, next: { $0.sheetParent }).contains { $0 === window }
            guard close else { return event }
            self.endSheet()
            return nil
        }
    }

    private func endSheet() {
        guard let sheet else { return }
        self.sheet = nil
        if let dismissMonitor { NSEvent.removeMonitor(dismissMonitor); self.dismissMonitor = nil }
        let display = sheet.display
        owner?.endSheet(sheet)
        owner = nil
        sheet.orderOut(nil)
        model.whatsNew.dismiss(display)
    }
}

/// The release highlights sheet: the face, title and installed version, then
/// each day's releases, and Done (or Got it for an automatic presentation).
/// Escape, Done or a click outside closes it; it is as tall as its notes.
@MainActor private final class AppKitWhatsNewSheet: NSWindow {
    private static let width: CGFloat = 520
    private let notes: WhatsNewModel
    private(set) var display: WhatsNewDisplay
    private let finish: () -> Void
    private let observation = AppKitObservation()
    private let body = AppKitKit.stack([], spacing: 16)
    private var scroll: NSScrollView!
    private var scrollHeight: NSLayoutConstraint!

    init(notes: WhatsNewModel, display: WhatsNewDisplay, close: @escaping () -> Void) {
        self.notes = notes; self.display = display; self.finish = close
        super.init(contentRect: NSRect(x: 0, y: 0, width: Self.width, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        appearance = NSAppearance(named: CaperThemeStore.shared.active.isLight ? .aqua : .darkAqua)
        isReleasedWhenClosed = false
        contentView = makeContent()
        observation.start(reading: { [notes] in (notes.presentation?.id, notes.presentation?.releases?.count, notes.loading, notes.error) }) { [weak self] _ in
            self?.render()
        }
    }

    private func makeContent() -> NSView {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: Self.width, height: 400))
        root.wantsLayer = true
        root.layer?.backgroundColor = AppKitKit.surface.cgColor
        root.setAccessibilityElement(true)
        root.setAccessibilityRole(.group)
        root.setAccessibilityIdentifier("whats-new")

        let title = AppKitKit.label("What’s new", size: 20, weight: .bold, wraps: false)
        title.setAccessibilityRole(.staticText)
        var heading: [NSView] = [AppKitKit.image("caper-face", size: 36), title]
        heading.append(appKitVersionControl())
        let header = AppKitKit.stack(heading, spacing: 2)
        header.setCustomSpacing(10, after: heading[0])

        let scroll = appKitKitScroll(body, insets: NSEdgeInsets(top: 22, left: 22, bottom: 22, right: 22))
        self.scroll = scroll
        scrollHeight = scroll.heightAnchor.constraint(equalToConstant: 200)
        // Done is the one way out on screen; Escape and a click outside also close.
        let done = AppKitKitButton(display.automatic ? "Got it" : "Done", kind: .primary) { [weak self] in self?.finish() }
        done.keyEquivalent = "\r"
        done.setAccessibilityIdentifier("whats-new-done")
        let divider = AppKitKit.divider()
        for view in [header, divider, scroll, done] as [NSView] { root.addSubview(view) }
        NSLayoutConstraint.activate([
            header.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 22),
            header.trailingAnchor.constraint(lessThanOrEqualTo: root.trailingAnchor, constant: -22),
            header.topAnchor.constraint(equalTo: root.topAnchor, constant: 22),
            divider.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            divider.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 22),
            scroll.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: divider.bottomAnchor),
            scrollHeight,
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
        defer { fitHeight() }
        if let releases = display.releases {
            if releases.isEmpty {
                add(AppKitKit.label("No release highlights for this version yet.", color: AppKitKit.muted))
                if let url = ReleaseChanges.url(from: nil, to: InstalledVersion.current.revision) {
                    add(AppKitKitLink(ReleaseChanges.label, url: url, help: "Open this version’s changes on GitHub"), fill: false)
                }
            }
            let days = Dictionary(grouping: releases) { String($0.publishedAt.prefix(10)) }
            for day in days.keys.sorted(by: >) {
                let date = selectable(AppKitKit.label(releaseDayLabel(day), size: 12, weight: .bold, color: AppKitKit.muted))
                add(date)
                // Each version sits right under its date, as on web.
                body.setCustomSpacing(6, after: date)
                for release in days[day] ?? [] {
                    var rows: [NSView] = []
                    if let url = URL(string: "https://github.com/joswayski/caper/commit/\(release.revision)") {
                        rows.append(AppKitKitLink(release.version.map { "Version \($0)" } ?? String(release.revision.prefix(7)), url: url, size: 12))
                    }
                    for highlight in release.highlights {
                        let title = selectable(AppKitKit.label(highlight.title, size: 16, weight: .bold))
                        let text = selectable(AppKitKit.label(highlight.body, size: 14, color: AppKitKit.muted))
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

    /// Release notes can be selected and copied, like any text on web.
    private func selectable(_ label: NSTextField) -> NSTextField { label.isSelectable = true; return label }

    /// Tall enough for the notes, so a short list leaves no empty space under
    /// it; a long one scrolls within the parent window's height.
    func fitHeight() {
        guard let root = contentView, let document = scroll.documentView else { return }
        // Twice: wrapped labels settle their height once they know their width.
        root.layoutSubtreeIfNeeded(); root.layoutSubtreeIfNeeded()
        let chrome = root.bounds.height - scroll.frame.height
        let limit = min(680, (sheetParent?.frame.height ?? 720) - 40)
        let height = min(chrome + document.frame.height, max(limit, chrome + 120))
        scrollHeight.constant = height - chrome
        // Keep the top edge where it is, attached under the parent's title bar.
        var next = frame
        let size = frameRect(forContentRect: NSRect(x: 0, y: 0, width: Self.width, height: height)).size
        next.origin.y += next.height - size.height; next.size = size
        setFrame(next, display: true)
    }

    override func cancelOperation(_ sender: Any?) { finish() }
}
#endif
