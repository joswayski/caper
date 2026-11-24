#if os(iOS)
import UIKit
import Observation

/// The root's native release-notes and speaking-ring lifetime. Sampling is
/// independent of the foreground timer.
@MainActor final class UIKitReleaseNotesPresenter: UIViewController {
    private let model: AppModel
    private var generation = 0
    private var timer: Timer?
    private var refresh: Task<Void, Never>?
    private var refreshGeneration = 0
    private lazy var speaking = NativeVoiceSpeaking(voice: model.voice)
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
        let center = NotificationCenter.default
        if parent != nil {
            observe(generation)
            // tick() does nothing unless Caper is active, so neither does the timer.
            center.addObserver(self, selector: #selector(becameActive), name: UIApplication.didBecomeActiveNotification, object: nil)
            center.addObserver(self, selector: #selector(resigningActive), name: UIApplication.willResignActiveNotification, object: nil)
            if UIApplication.shared.applicationState == .active { becameActive() }
        } else {
            center.removeObserver(self, name: UIApplication.didBecomeActiveNotification, object: nil)
            center.removeObserver(self, name: UIApplication.willResignActiveNotification, object: nil)
            refreshGeneration += 1
            timer?.invalidate(); timer = nil; refresh?.cancel(); refresh = nil
            speaking.stop(); sheet?.dismiss(animated: false); sheet = nil
        }
    }
    isolated deinit { timer?.invalidate(); refresh?.cancel() }
    @objc private func becameActive() {
        guard timer == nil else { return }
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.tick() } }
    }
    /// What an inactive tick() did: stop a refresh, and start afresh next time.
    @objc private func resigningActive() {
        timer?.invalidate(); timer = nil
        refreshGeneration += 1; refresh?.cancel(); refresh = nil; wasActive = false
    }
    override func viewDidAppear(_ animated: Bool) { super.viewDidAppear(animated); tick(); sync() }
    private func observe(_ token: Int) {
        guard token == generation, parent != nil else { return }
        withObservationTracking {
            let ready = model.phase == .ready
            if ready && !wasReady { CaperRuntime.showWhatsNewPreview(model) }
            wasReady = ready
            speaking.sync(); sync()
        } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } }
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
        // As tall as the notes (a tap above a short sheet closes it), and
        // draggable to full height when they run longer.
        let fit = UISheetPresentationController.Detent.custom(identifier: UIKitReleaseNotes.fit) { [weak sheet, weak self] context in
            let maximum = context.maximumDetentValue
            return MainActor.assumeIsolated {
                guard let sheet else { return nil }
                let width = sheet.view.bounds.width > 0 ? sheet.view.bounds.width : self?.view.window?.bounds.width ?? 390
                return min(sheet.fittingHeight(width: width), maximum)
            }
        }
        sheet.sheetPresentationController?.detents = [fit, .large()]
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
    static let fit = UISheetPresentationController.Detent.Identifier("whats-new-fit")
    private let notes: WhatsNewModel
    private(set) var display: WhatsNewDisplay
    /// Kept across renders so its "Copied" feedback and focus survive.
    private let version = UIKitVersionControl()
    init(notes: WhatsNewModel, display: WhatsNewDisplay, close: @escaping () -> Void) {
        self.notes = notes; self.display = display; super.init(title: "What’s new", closeLabel: "Close What’s new", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    /// Done, a swipe down or a tap outside closes it.
    override var showsCloseButton: Bool { false }
    override func viewDidLoad() { super.viewDidLoad(); view.accessibilityIdentifier = "whats-new"; observe() }
    override func render() {
        if display.releases == nil, let next = notes.presentation, next.id == display.id { display = next }
        body.arrangedSubviews.forEach { body.removeArrangedSubview($0); $0.removeFromSuperview() }
        let face = UIImageView(image: UIImage(named: "caper-face", in: caperResourceBundle, compatibleWith: nil))
        face.contentMode = .scaleAspectFit; face.isAccessibilityElement = false
        // The face sits on the leading edge, as on the other clients, not
        // centred across the sheet; the version follows right under it.
        let faceRow = UIView(); face.translatesAutoresizingMaskIntoConstraints = false; faceRow.addSubview(face)
        NSLayoutConstraint.activate([
            face.widthAnchor.constraint(equalToConstant: 36), face.heightAnchor.constraint(equalToConstant: 36),
            face.leadingAnchor.constraint(equalTo: faceRow.leadingAnchor), face.topAnchor.constraint(equalTo: faceRow.topAnchor),
            face.bottomAnchor.constraint(equalTo: faceRow.bottomAnchor),
        ])
        body.addArrangedSubview(faceRow); body.setCustomSpacing(6, after: faceRow)
        body.addArrangedSubview(version); body.setCustomSpacing(20, after: version)
        if let releases = display.releases {
            if releases.isEmpty {
                let empty = Self.label("No release highlights for this version yet.")
                body.addArrangedSubview(empty)
                if let url = ReleaseChanges.url(from: nil, to: InstalledVersion.current.revision) {
                    body.setCustomSpacing(8, after: empty)
                    body.addArrangedSubview(link(ReleaseChanges.label, url: url, hint: "Open this version’s changes on GitHub"))
                }
            }
            let days = Dictionary(grouping: releases) { String($0.publishedAt.prefix(10)) }
            for date in days.keys.sorted(by: >) {
                let day = Self.selectable(releaseDayLabel(date), size: 12, bold: true, muted: true)
                body.addArrangedSubview(day); body.setCustomSpacing(4, after: day)
                for release in days[date] ?? [] {
                    // Web's spacing: the link 6 above its highlights, each title
                    // 4 above its text, 12 between highlights, 20 around the rule.
                    let source = link(release.version.map { "Version \($0)" } ?? String(release.revision.prefix(7)),
                                      url: URL(string: "https://github.com/joswayski/caper/commit/\(release.revision)"))
                    body.addArrangedSubview(source); body.setCustomSpacing(6, after: source)
                    for highlight in release.highlights {
                        let title = Self.selectable(highlight.title, size: 16, bold: true); title.accessibilityTraits.insert(.header)
                        let text = Self.selectable(highlight.body, size: 14, muted: true)
                        body.addArrangedSubview(title); body.setCustomSpacing(4, after: title)
                        body.addArrangedSubview(text); body.setCustomSpacing(12, after: text)
                    }
                    if let last = body.arrangedSubviews.last { body.setCustomSpacing(20, after: last) }
                    let rule = UIView(); rule.backgroundColor = UIColor(CaperTheme.border); rule.heightAnchor.constraint(equalToConstant: 1).isActive = true
                    body.addArrangedSubview(rule); body.setCustomSpacing(20, after: rule)
                }
            }
        } else if notes.loading { body.addArrangedSubview(Self.label("Loading release highlights…")) }
        else if let error = notes.error {
            body.addArrangedSubview(Self.label(error, error: true))
            body.addArrangedSubview(Self.button("Retry") { [notes] in Task { await notes.refresh() } })
        }
        body.addArrangedSubview(Self.button(display.automatic ? "Got it" : "Done", primary: true, action: close))
        if view.window != nil, let sheet = sheetPresentationController { sheet.animateChanges { sheet.invalidateDetents() } }
    }
    /// Release notes can be selected and copied, like text on web.
    private static func selectable(_ text: String, size: CGFloat, bold: Bool = false, muted: Bool = false) -> UITextView {
        let view = UITextView()
        view.text = text; view.isEditable = false; view.isSelectable = true; view.isScrollEnabled = false
        view.backgroundColor = .clear; view.textContainerInset = .zero; view.textContainer.lineFragmentPadding = 0
        view.font = UIFontMetrics(forTextStyle: .body).scaledFont(for:
            UIFont(name: bold ? "Figtree-Bold" : "Figtree-Regular", size: size) ?? .systemFont(ofSize: size))
        view.adjustsFontForContentSizeCategory = true
        view.textColor = UIColor(muted ? CaperTheme.muted : CaperTheme.text)
        return view
    }
    /// A quiet commit link like every other client's: flush with the text,
    /// muted at rest with an external-link icon, brightening while pressed.
    private func link(_ title: String, url: URL?, hint: String = "View this version on GitHub") -> UIButton {
        var config = UIButton.Configuration.plain()
        config.title = title
        config.contentInsets = NSDirectionalEdgeInsets(top: 2, leading: 0, bottom: 2, trailing: 0)
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
            var attributes = incoming
            attributes.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: "Figtree-Regular", size: 12) ?? .systemFont(ofSize: 12))
            return attributes
        }
        if let icon = UIImage(named: "caper-external-link") {
            config.image = UIGraphicsImageRenderer(size: CGSize(width: 12, height: 12)).image { _ in
                icon.draw(in: CGRect(x: 0, y: 0, width: 12, height: 12))
            }.withRenderingMode(.alwaysTemplate)
            config.imagePlacement = .trailing; config.imagePadding = 4
        }
        let button = UIKitTallTapButton(configuration: config, primaryAction: UIAction { _ in
            if let url { UIApplication.shared.open(url) }
        })
        button.configurationUpdateHandler = { button in
            button.configuration?.baseForegroundColor = button.isHighlighted ? UIColor(CaperTheme.text) : UIColor(CaperTheme.muted).withAlphaComponent(0.72)
        }
        button.accessibilityHint = hint
        button.contentHorizontalAlignment = .leading
        return button
    }
}

/// "Version 2026.11.19.3", as on every client: a tap (or Return or Space with a
/// keyboard) copies "Caper 2026.11.19.3 for iOS (5c00469)" and the label reads
/// "Copied" for two seconds, announced to VoiceOver. Development builds show
/// "Development build" and can't be pressed.
@MainActor final class UIKitVersionControl: UIButton {
    private let installed: InstalledVersion
    private var generation = 0

    init(_ installed: InstalledVersion = .current) {
        self.installed = installed
        super.init(frame: .zero)
        var config = UIButton.Configuration.plain()
        config.contentInsets = NSDirectionalEdgeInsets(top: 2, leading: 0, bottom: 2, trailing: 0)
        config.baseForegroundColor = UIColor(CaperTheme.text)
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
            var attributes = incoming
            attributes.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: "Figtree-Regular", size: 13) ?? .systemFont(ofSize: 13))
            return attributes
        }
        config.title = installed.label
        configuration = config
        contentHorizontalAlignment = .leading
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
    /// Drawn as tall as its text, tapped like a 44 pt control.
    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        bounds.insetBy(dx: 0, dy: min(0, (bounds.height - 44) / 2)).contains(point)
    }

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
/// A link drawn as tall as its text that still takes taps across 44 pt, so
/// stacked links keep their text's spacing without shrinking the target.
@MainActor final class UIKitTallTapButton: UIButton {
    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        bounds.insetBy(dx: 0, dy: min(0, (bounds.height - 44) / 2)).contains(point)
    }
}
#endif
