#if os(macOS)
import AppKit
import ServiceManagement
import UniformTypeIdentifiers

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
            window.onCancel = { [weak self] in self?.dismiss() }
            let loginItem = loginItem ?? CaperLoginItem(available: false, status: { .notRegistered }, register: {}, unregister: {})
            window.contentViewController = AppKitSettingsController(
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

/// A compact labeled color swatch picker for live theme customization.
@MainActor private final class ThemeColorSwatchRow: NSView {
    let well: NSColorWell
    private let label: NSTextField
    private let onPick: (NSColor) -> Void

    init(title: String, color: NSColor, onPick: @escaping (NSColor) -> Void) {
        self.onPick = onPick
        well = NSColorWell(frame: NSRect(x: 0, y: 0, width: 34, height: 24))
        well.color = color
        if #available(macOS 13.0, *) {
            well.colorWellStyle = .minimal
        }
        label = AppKitKit.label(title, size: 12, weight: .medium, wraps: false)
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        well.translatesAutoresizingMaskIntoConstraints = false
        well.target = self
        well.action = #selector(colorChanged)
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(well)
        addSubview(label)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 28),
            well.leadingAnchor.constraint(equalTo: leadingAnchor),
            well.centerYAnchor.constraint(equalTo: centerYAnchor),
            well.widthAnchor.constraint(equalToConstant: 36),
            well.heightAnchor.constraint(equalToConstant: 24),
            label.leadingAnchor.constraint(equalTo: well.trailingAnchor, constant: 8),
            label.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func setHex(_ hex: String) {
        if let color = CaperThemeStore.nsColor(fromHex: hex), well.color != color {
            well.color = color
        }
    }

    @objc private func colorChanged() {
        onPick(well.color)
    }
}

/// Startup, appearance/theme customizer, sounds, voice messages, updates and release notes.
@MainActor private final class AppKitSettingsController: NSViewController {
    private let whatsNew: WhatsNewModel
    private let loginItem: CaperLoginItem
    private let updater: AppUpdater?
    private let onClose: () -> Void
    private let effects = CaperEffects.shared
    private let voiceMessages = VoiceMessageSettings.shared
    private let themeStore = CaperThemeStore.shared
    private let observation = AppKitObservation()
    private var draftTheme: CaperThemeDocument

    private lazy var closeButton = AppKitKitButton("Done", kind: .secondary) { [weak self] in self?.onClose() }
    private lazy var themeSelect = AppKitCaperSelect(items: ["Obsidian (Dark)", "Paper (Light)"]) { [weak self] idx in
        guard let self else { return }
        let themes = self.themeStore.allThemes
        guard themes.indices.contains(idx) else { return }
        let picked = themes[idx]
        self.themeStore.selectPreset(picked.id)
        self.draftTheme = self.themeStore.active
        self.themeStatus.stringValue = ""
        self.syncThemeEditor()
    }
    private lazy var themeNameField = AppKitKitField(height: 32, size: 12.5)
    private lazy var darkModeBtn = AppKitKitButton("Dark") { [weak self] in self?.setDraftAppearance("dark") }
    private lazy var lightModeBtn = AppKitKitButton("Light") { [weak self] in self?.setDraftAppearance("light") }
    private lazy var bgSwatch = ThemeColorSwatchRow(title: "Background", color: AppKitKit.surface) { [weak self] c in self?.updateDraftColor(\.surfaces.frame, also: \.surfaces.base, color: c) }
    private lazy var sidebarSwatch = ThemeColorSwatchRow(title: "Sidebar", color: CaperThemeStore.sidebarColor) { [weak self] c in self?.updateDraftColor(\.surfaces.sidebar, color: c) }
    private lazy var chatSwatch = ThemeColorSwatchRow(title: "Conversation", color: CaperThemeStore.conversationColor) { [weak self] c in self?.updateDraftColor(\.surfaces.conversation, color: c) }
    private lazy var cardSwatch = ThemeColorSwatchRow(title: "Cards & Input", color: AppKitKit.raised) { [weak self] c in self?.updateDraftColor(\.surfaces.raised, also: \.surfaces.composer, color: c) }
    private lazy var accentSwatch = ThemeColorSwatchRow(title: "Accent", color: AppKitKit.terracotta) { [weak self] c in self?.updateDraftColor(\.accent.primary, also: \.accent.bright, color: c) }
    private lazy var actionSwatch = ThemeColorSwatchRow(title: "Primary Button", color: AppKitKit.action) { [weak self] c in self?.updatePrimaryActionColor(c) }
    private lazy var textSwatch = ThemeColorSwatchRow(title: "Text", color: AppKitKit.text) { [weak self] c in self?.updateDraftColor(\.text.primary, also: \.text.message, color: c) }
    private lazy var borderSwatch = ThemeColorSwatchRow(title: "Borders", color: AppKitKit.border) { [weak self] c in self?.updateDraftColor(\.borders.default, color: c) }

    private lazy var saveCustomTheme = AppKitKitButton("Save theme", kind: .primary) { [weak self] in self?.saveDraftCustomTheme() }
    private lazy var deleteCustomTheme = AppKitKitButton("Delete", kind: .destructive) { [weak self] in self?.deleteActiveCustomTheme() }
    private lazy var resetTheme = AppKitKitButton("Reset") { [weak self] in self?.resetDraftTheme() }
    private lazy var importTheme = AppKitKitButton("Import JSON") { [weak self] in self?.chooseThemeFile() }
    private lazy var exportTheme = AppKitKitButton("Export JSON") { [weak self] in self?.saveThemeFile() }
    private let themeStatus = AppKitKit.label("", size: 12, color: AppKitKit.muted)

    private lazy var launch = AppKitKitSwitchRow(title: "Launch at login", detail: "Open Caper when you sign in to your computer.",
                                                 identifier: "launch-at-login") { [weak self] on in self?.loginItem.setEnabled(on) }
    private let startupNotes = AppKitKit.stack([], spacing: 8)
    private lazy var sounds = AppKitKitSwitchRow(title: "Caper sound effects", detail: "Play sounds for messages and app interactions.",
                                                 identifier: "sound-effects") { [weak self] on in self?.effects.soundsEnabled = on }
    private lazy var sendOnStop = AppKitKitSwitchRow(title: VoiceMessage.sendOnStopTitle, detail: VoiceMessage.sendOnStopDetail,
                                                     identifier: "voice-messages-send-on-stop") { [weak self] on in self?.voiceMessages.sendOnStop = on }
    private let version = NSStackView()
    private let status = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private lazy var check = AppKitKitButton("Check for updates") { [weak self] in self?.controls.check?() }
    private lazy var notes = AppKitKitButton("What’s new") { [weak self] in self?.whatsNew.open() }

    init(whatsNew: WhatsNewModel, loginItem: CaperLoginItem, updater: AppUpdater?, onClose: @escaping () -> Void) {
        self.whatsNew = whatsNew; self.loginItem = loginItem; self.updater = updater; self.onClose = onClose
        self.draftTheme = CaperThemeStore.shared.active
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
        themeSelect.setAccessibilityIdentifier("appearance-theme-select")
        themeSelect.setAccessibilityLabel("Theme")
        closeButton.setAccessibilityIdentifier("close-settings")
        saveCustomTheme.minimumHeight = 32
        themeNameField.placeholderString = "Custom theme name"
        version.orientation = .horizontal; version.spacing = 4
        // Built once, so its "Copied" feedback and keyboard focus survive renders.
        version.addArrangedSubview(appKitVersionControl())
        func section(_ title: String, _ rows: [NSView]) -> NSStackView {
            AppKitKit.stack([AppKitKit.label(title, size: 12, weight: .bold, color: AppKitKit.muted)] + rows, spacing: 12)
        }

        let header = NSStackView(views: [AppKitKit.label("Settings", size: 20, weight: .bold, wraps: false), NSView(), closeButton])
        header.orientation = .horizontal; header.alignment = .centerY; header.translatesAutoresizingMaskIntoConstraints = false

        let nameAndMode = NSStackView(views: [themeNameField, darkModeBtn, lightModeBtn])
        nameAndMode.orientation = .horizontal; nameAndMode.spacing = 8; nameAndMode.alignment = .centerY
        let leftCol = AppKitKit.stack([bgSwatch, sidebarSwatch, chatSwatch, cardSwatch], spacing: 6)
        let rightCol = AppKitKit.stack([accentSwatch, actionSwatch, textSwatch, borderSwatch], spacing: 6)
        let swatchGrid = NSStackView(views: [leftCol, rightCol])
        swatchGrid.orientation = .horizontal; swatchGrid.distribution = .fillEqually; swatchGrid.spacing = 16
        let customActions = NSStackView(views: [saveCustomTheme, deleteCustomTheme, resetTheme])
        customActions.orientation = .horizontal; customActions.spacing = 8
        let fileActions = NSStackView(views: [importTheme, exportTheme])
        fileActions.orientation = .horizontal; fileActions.spacing = 8
        let customLabel = AppKitKit.label("Customize colors (live preview)", size: 12, weight: .medium, color: AppKitKit.muted)
        let appearance = section("Appearance", [themeSelect, customLabel, nameAndMode, swatchGrid, customActions, fileActions, themeStatus])

        let buttons = NSStackView(views: [check, notes]); buttons.orientation = .horizontal; buttons.spacing = 8
        let updates = section("Updates", [version, buttons, status])
        let content = AppKitKit.stack([
            header,
            appearance, AppKitKit.divider(),
            section("Startup", [launch, startupNotes]), AppKitKit.divider(),
            section("Sounds", [sounds]), AppKitKit.divider(),
            section("Voice messages", [sendOnStop]), AppKitKit.divider(),
            updates,
        ], spacing: 18)
        for case let child as NSStackView in content.arrangedSubviews { AppKitKit.fillWidth(child, of: content) }
        for view in content.arrangedSubviews where !(view is NSStackView) && !(view is NSButton) && !(view is NSTextField) {
            AppKitKit.fillWidth(view, of: content)
        }
        for row in [themeSelect, nameAndMode, swatchGrid, launch, sounds, sendOnStop] as [NSView] {
            if let stack = row.superview as? NSStackView { AppKitKit.fillWidth(row, of: stack) }
        }
        let scroll = appKitKitScroll(content, insets: NSEdgeInsets(top: 22, left: 24, bottom: 24, right: 24))
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
        syncThemeEditor()
    }

    override func viewWillAppear() {
        super.viewWillAppear()
        refreshLoginItem()
        draftTheme = themeStore.active
        syncThemeEditor()
    }

    func refreshLoginItem() { loginItem.refresh() }
    @objc private func applicationBecameActive() { loginItem.refresh() }
    @objc private func handleThemeChange() {
        view.layer?.backgroundColor = AppKitKit.surface.cgColor
        view.window?.backgroundColor = AppKitKit.surface
        view.window?.appearance = NSAppearance(named: themeStore.active.isLight ? .aqua : .darkAqua)
        view.needsDisplay = true
    }

    private func setDraftAppearance(_ appearance: String) {
        guard draftTheme.appearance != appearance else { return }
        let base = appearance == "light" ? CaperThemeDocument.paper : CaperThemeDocument.obsidian
        let keepAccent = draftTheme.accent
        let keepBrand = draftTheme.brand
        let keepID = (draftTheme.id == "obsidian" || draftTheme.id == "paper") ? "custom-draft" : draftTheme.id
        let typedName = themeNameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        let keepName = (draftTheme.id == "obsidian" || draftTheme.id == "paper")
            ? (typedName.isEmpty || typedName == "Obsidian" || typedName == "Paper" ? "My Theme" : typedName)
            : draftTheme.name
        var next = base
        next.id = keepID
        next.name = keepName
        next.accent = keepAccent
        next.brand = keepBrand
        draftTheme = next
        themeStore.applyLiveTheme(draftTheme, persist: true)
        syncThemeEditor()
    }

    private func updateDraftColor(_ keyPath: WritableKeyPath<CaperThemeDocument, String>,
                                  also secondary: WritableKeyPath<CaperThemeDocument, String>? = nil,
                                  color: NSColor) {
        let hex = CaperThemeStore.hexString(from: color)
        draftTheme[keyPath: keyPath] = hex
        if let secondary { draftTheme[keyPath: secondary] = hex }
        if draftTheme.id == "obsidian" || draftTheme.id == "paper" {
            draftTheme.id = "custom-draft"
            if themeNameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || themeNameField.stringValue == "Obsidian" || themeNameField.stringValue == "Paper" {
                draftTheme.name = "My Theme"
            }
        }
        themeStore.applyLiveTheme(draftTheme, persist: true)
        syncThemeEditor()
    }

    private func updatePrimaryActionColor(_ color: NSColor) {
        let hex = CaperThemeStore.hexString(from: color)
        draftTheme.action.primary = hex
        draftTheme.action.primaryHover = hex
        draftTheme.action.primaryPress = hex
        draftTheme.action.onPrimary = CaperThemeStore.readableForegroundHex(forHex: hex)
        if draftTheme.id == "obsidian" || draftTheme.id == "paper" {
            draftTheme.id = "custom-draft"
            draftTheme.name = "My Theme"
        }
        themeStore.applyLiveTheme(draftTheme, persist: true)
        syncThemeEditor()
    }

    private func saveDraftCustomTheme() {
        let entered = themeNameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        draftTheme.name = entered.isEmpty || entered == "Obsidian" || entered == "Paper" ? "My Custom Theme" : entered
        if draftTheme.id == "obsidian" || draftTheme.id == "paper" || draftTheme.id == "custom-draft" {
            draftTheme.id = ""
        }
        themeStore.saveCustomTheme(draftTheme)
        draftTheme = themeStore.active
        themeStatus.textColor = AppKitKit.terracottaBright
        themeStatus.stringValue = "Saved \"\(draftTheme.name)\"."
        themeStatus.isHidden = false
        syncThemeEditor()
    }

    private func deleteActiveCustomTheme() {
        guard themeStore.isCustom else { return }
        let removed = themeStore.active.name
        themeStore.deleteCustomTheme(id: themeStore.active.id)
        draftTheme = themeStore.active
        themeStatus.textColor = AppKitKit.muted
        themeStatus.stringValue = "Deleted \"\(removed)\"."
        themeStatus.isHidden = false
        syncThemeEditor()
    }

    private func resetDraftTheme() {
        let baseId = draftTheme.isLight ? "paper" : "obsidian"
        themeStore.selectPreset(baseId)
        draftTheme = themeStore.active
        themeStatus.textColor = AppKitKit.muted
        themeStatus.stringValue = "Reset to \(draftTheme.name)."
        themeStatus.isHidden = false
        syncThemeEditor()
    }

    private func syncThemeEditor() {
        if themeNameField.stringValue != draftTheme.name {
            themeNameField.stringValue = draftTheme.name
        }
        darkModeBtn.showsSelection = !draftTheme.isLight
        lightModeBtn.showsSelection = draftTheme.isLight
        bgSwatch.setHex(draftTheme.surfaces.base)
        sidebarSwatch.setHex(draftTheme.surfaces.sidebar)
        chatSwatch.setHex(draftTheme.surfaces.conversation)
        cardSwatch.setHex(draftTheme.surfaces.raised)
        accentSwatch.setHex(draftTheme.accent.primary)
        actionSwatch.setHex(draftTheme.action.primary)
        textSwatch.setHex(draftTheme.text.primary)
        borderSwatch.setHex(draftTheme.borders.default)
        deleteCustomTheme.isHidden = !themeStore.savedThemes.contains(where: { $0.id == themeStore.active.id })
    }

    private func chooseThemeFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.message = "Choose a .capertheme.json file"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            let text = try String(contentsOf: url, encoding: .utf8)
            try themeStore.importThemeJSON(text)
            draftTheme = themeStore.active
            syncThemeEditor()
            themeStatus.textColor = AppKitKit.terracottaBright
            themeStatus.stringValue = "Applied \"\(themeStore.active.name)\"."
        } catch {
            themeStatus.textColor = AppKitKit.error
            themeStatus.stringValue = error.localizedDescription
        }
        themeStatus.isHidden = themeStatus.stringValue.isEmpty
    }

    private func saveThemeFile() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(themeStore.active.id).capertheme.json"
        panel.allowedContentTypes = [.json]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try themeStore.exportCurrentJSON().write(to: url, atomically: true, encoding: .utf8)
            themeStatus.textColor = AppKitKit.muted
            themeStatus.stringValue = "Exported \(url.lastPathComponent)."
        } catch {
            themeStatus.textColor = AppKitKit.error
            themeStatus.stringValue = error.localizedDescription
        }
        themeStatus.isHidden = themeStatus.stringValue.isEmpty
    }

    /// Reads every observed value the window shows.
    private func snapshot() -> [AnyHashable] {
        let c = controls
        return [loginItem.registered, loginItem.available, loginItem.status == .requiresApproval, loginItem.error,
                effects.soundsEnabled, voiceMessages.sendOnStop, themeStore.active.id, themeStore.active.name,
                themeStore.savedThemes.count,
                c.isChecking, c.review != nil, c.check != nil, c.lastSuccessfulCheck, c.installedVersion, c.sourceRevision]
    }

    private func render() {
        view.layer?.backgroundColor = AppKitKit.surface.cgColor
        let themes = themeStore.allThemes
        var labels = themes.map { doc -> String in
            if doc.id == "obsidian" { return "Obsidian (Dark)" }
            if doc.id == "paper" { return "Paper (Light)" }
            return "\(doc.name) (\(doc.isLight ? "Light" : "Dark"))"
        }
        if let idx = themes.firstIndex(where: { $0.id == themeStore.active.id }) {
            themeSelect.items = labels
            themeSelect.selectedIndex = idx
        } else {
            labels.append("Unsaved: \(themeStore.active.name)")
            themeSelect.items = labels
            themeSelect.selectedIndex = labels.count - 1
        }
        themeStatus.isHidden = themeStatus.stringValue.isEmpty
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
        var wasReady = false
        readiness.start(reading: { [model] in model.phase == .ready }) { [model] isReady in
            if isReady && !wasReady { CaperRuntime.showWhatsNewPreview(model) }
            wasReady = isReady
        }
        // tick() does nothing while another app is active, so neither does the timer.
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(activityChanged), name: NSApplication.didBecomeActiveNotification, object: nil)
        center.addObserver(self, selector: #selector(activityChanged), name: NSApplication.didResignActiveNotification, object: nil)
        activityChanged()
    }

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
