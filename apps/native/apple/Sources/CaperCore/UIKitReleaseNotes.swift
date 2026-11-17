#if os(iOS)
import UIKit
import Observation

/// The root's native release-notes and call-effects lifetime. Sampling is
/// independent of the foreground timer so an active call keeps its chimes.
@MainActor final class UIKitReleaseNotesPresenter: UIViewController {
    private let model: AppModel
    private var generation = 0
    private var timer: Timer?
    private var refresh: Task<Void, Never>?
    private var refreshGeneration = 0
    private var speaking: Task<Void, Never>?
    private var phase: VoiceClient.Phase?
    private var participants: [String] = []
    private var announced = false
    private var wasReady = false
    private var wasActive = false
    private var lastRefresh: Date?
    private var sheet: UIKitReleaseNotes?
    private var closing = false
    init(model: AppModel) { self.model = model; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() { view = UIView(); view.isUserInteractionEnabled = false; view.accessibilityElementsHidden = true }
    override func didMove(toParent parent: UIViewController?) {
        super.didMove(toParent: parent); generation += 1
        if parent != nil {
            CaperEffects.shared.preload(); observe(generation)
            timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.tick() } }
        } else {
            refreshGeneration += 1
            timer?.invalidate(); timer = nil; refresh?.cancel(); refresh = nil; speaking?.cancel(); speaking = nil
            model.voice.clearSpeaking(); sheet?.dismiss(animated: false); sheet = nil
        }
    }
    isolated deinit { timer?.invalidate(); refresh?.cancel(); speaking?.cancel() }
    override func viewDidAppear(_ animated: Bool) { super.viewDidAppear(animated); tick(); sync() }
    private func observe(_ token: Int) {
        guard token == generation, parent != nil else { return }
        withObservationTracking {
            let ready = model.phase == .ready
            if ready && !wasReady { CaperRuntime.showWhatsNewPreview(model) }
            wasReady = ready
            syncEffects(); sync()
        } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } }
    }
    private func syncEffects() {
        let voice = model.voice
        let next = voice.participants.map(\.id)
        if next != participants, voice.phase == .connected,
           let me = participants.first(where: { voice.isSelf(participantID: $0) }), next.contains(me) {
            let before = Set(participants).subtracting([me]), after = Set(next).subtracting([me])
            if !before.subtracting(after).isEmpty { CaperEffects.shared.play(.leave) }
            else if !after.subtracting(before).isEmpty { CaperEffects.shared.play(.join) }
        }
        participants = next
        guard phase != voice.phase else { return }
        phase = voice.phase; speaking?.cancel(); speaking = nil
        if voice.phase == .idle || voice.phase == .failed || voice.phase == .joining { announced = false }
        if voice.phase == .connected {
            if !announced { announced = true; CaperEffects.shared.play(.join) }
            speaking = Task { [weak voice] in
                while !Task.isCancelled, let voice, voice.phase == .connected {
                    await voice.sampleSpeaking()
                    do { try await Task.sleep(for: .milliseconds(100)) } catch { return }
                }
            }
        } else { voice.clearSpeaking() }
    }
    private func tick() {
        let active = view.window?.windowScene?.activationState == .foregroundActive && model.phase == .ready
            && model.account != nil && model.spacesLoaded && !model.restoringWorkspace
        defer { wasActive = active }
        guard active else { refreshGeneration += 1; refresh?.cancel(); refresh = nil; return }
        if !wasActive { lastRefresh = nil }
        let notes = model.whatsNew
        if !notes.hasSnapshot, refresh == nil, lastRefresh.map({ Date().timeIntervalSince($0) >= 60 }) ?? true {
            lastRefresh = Date()
            refreshGeneration += 1; let token = refreshGeneration
            refresh = Task { [weak self] in
                await notes.refresh()
                guard let self, self.refreshGeneration == token else { return }; self.refresh = nil
            }
        }
        if (model.voice.phase == .idle || model.voice.phase == .failed), !notes.workspaceModalOpen,
           !notes.settingsOpen, !WhatsNewModel.nativeModalOpen { notes.presentAutomatically() }
        sync()
    }
    private func sync() {
        let display = model.whatsNew.presentation
        guard !closing else { return }
        if display == nil { if sheet != nil { close() }; return }
        guard sheet == nil, view.window != nil, presentedViewController == nil,
              parent?.presentedViewController == nil, let display else { return }
        let sheet = UIKitReleaseNotes(notes: model.whatsNew, display: display) { [weak self] in self?.close() }
        self.sheet = sheet
        UIKitHosting.sheet(sheet, background: CaperTheme.surface)
        sheet.sheetPresentationController?.detents = [.large()]
        present(sheet, animated: !CaperAccessibility.reduceMotion)
    }
    private func close() {
        guard !closing, let sheet else { return }
        closing = true; self.sheet = nil; model.whatsNew.dismiss(sheet.display); sheet.stop()
        let complete: () -> Void = { [weak self] in self?.closing = false; self?.sync() }
        if let presenter = sheet.presentingViewController { presenter.dismiss(animated: false, completion: complete) } else { complete() }
    }
}

@MainActor private final class UIKitReleaseNotes: UIKitMessageDialog {
    private let notes: WhatsNewModel
    private(set) var display: WhatsNewDisplay
    /// Kept across renders so its "Copied" feedback and focus survive.
    private let version = UIKitVersionControl()
    init(notes: WhatsNewModel, display: WhatsNewDisplay, close: @escaping () -> Void) {
        self.notes = notes; self.display = display; super.init(title: "What’s new", closeLabel: "Close What’s new", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() { super.viewDidLoad(); view.accessibilityIdentifier = "whats-new"; observe() }
    override func render() {
        if display.releases == nil, let next = notes.presentation, next.id == display.id { display = next }
        body.arrangedSubviews.forEach { body.removeArrangedSubview($0); $0.removeFromSuperview() }
        let face = UIImageView(image: UIImage(named: "caper-face", in: caperResourceBundle, compatibleWith: nil))
        face.contentMode = .scaleAspectFit; face.heightAnchor.constraint(equalToConstant: 36).isActive = true; face.isAccessibilityElement = false
        body.addArrangedSubview(face)
        body.addArrangedSubview(version)
        if let releases = display.releases {
            if releases.isEmpty { body.addArrangedSubview(Self.label("No release highlights for this version yet.")) }
            let days = Dictionary(grouping: releases) { String($0.publishedAt.prefix(10)) }
            for date in days.keys.sorted(by: >) {
                body.addArrangedSubview(Self.label(releaseDayLabel(date), bold: true))
                for release in days[date] ?? [] {
                    body.addArrangedSubview(link(release.version.map { "Version \($0)" } ?? String(release.revision.prefix(7)), revision: release.revision))
                    for highlight in release.highlights {
                        let title = Self.label(highlight.title, size: 16, bold: true); title.accessibilityTraits.insert(.header)
                        body.addArrangedSubview(title); body.addArrangedSubview(Self.label(highlight.body, size: 14))
                    }
                    let rule = UIView(); rule.backgroundColor = UIColor(CaperTheme.border); rule.heightAnchor.constraint(equalToConstant: 1).isActive = true
                    body.addArrangedSubview(rule)
                }
            }
        } else if notes.loading { body.addArrangedSubview(Self.label("Loading release highlights…")) }
        else if let error = notes.error {
            body.addArrangedSubview(Self.label(error, error: true))
            body.addArrangedSubview(Self.button("Retry") { [notes] in Task { await notes.refresh() } })
        }
        body.addArrangedSubview(Self.button(display.automatic ? "Got it" : "Done", primary: true, action: close))
    }
    private func link(_ title: String, revision: String) -> UIButton {
        let button = Self.button(title) {
            if let url = URL(string: "https://github.com/joswayski/caper/commit/\(revision)") { UIApplication.shared.open(url) }
        }
        button.accessibilityHint = "View this version on GitHub"
        button.contentHorizontalAlignment = .leading
        return button
    }
}

/// "Version 0.1.654", as on every client: a tap (or Return or Space with a
/// keyboard) copies "Caper 0.1.654 for iOS (5c00469)" and the label reads
/// "Copied" for two seconds, announced to VoiceOver. Development builds show
/// "Development build" and can't be pressed.
@MainActor final class UIKitVersionControl: UIButton {
    private let installed: InstalledVersion
    private var generation = 0

    init(_ installed: InstalledVersion = .current) {
        self.installed = installed
        super.init(frame: .zero)
        var config = UIButton.Configuration.plain()
        config.contentInsets = NSDirectionalEdgeInsets(top: 4, leading: 0, bottom: 4, trailing: 0)
        config.baseForegroundColor = UIColor(CaperTheme.text)
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
            var attributes = incoming
            attributes.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: "Satoshi-Regular", size: 13) ?? .systemFont(ofSize: 13))
            return attributes
        }
        config.title = installed.label
        configuration = config
        contentHorizontalAlignment = .leading
        heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        accessibilityIdentifier = "copy-version"
        accessibilityLabel = installed.accessibilityLabel
        if installed.copyText == nil {
            isUserInteractionEnabled = false; accessibilityTraits = .staticText
        } else {
            accessibilityHint = InstalledVersion.hint
            addAction(UIAction { [weak self] _ in self?.copyVersion() }, for: .primaryActionTriggered)
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func copyVersion() {
        guard let copy = installed.copyText else { return }
        UIPasteboard.general.string = copy
        generation += 1
        let current = generation
        configuration?.title = InstalledVersion.copied
        UIAccessibility.post(notification: .announcement, argument: "Copied \(copy)")
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(InstalledVersion.feedbackSeconds))
            guard let self, self.generation == current else { return }
            self.configuration?.title = self.installed.label
        }
    }
}
#endif
