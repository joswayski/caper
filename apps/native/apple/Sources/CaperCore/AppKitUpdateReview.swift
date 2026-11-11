#if os(macOS)
import AppKit

/// The desktop update review over the account window: a scrim, then a card
/// with the version, release highlights and the install action. The account
/// UI stays inert underneath until it closes.
@MainActor public final class AppKitUpdateReview: NSView {
    private let update: DesktopUpdate
    private let inCall: Bool
    private let close: () -> Void
    private let restart: () -> Void
    private let download: () -> Void
    private let report: ((String) async -> String?)?
    private let card = NSView()
    private let reportArea = AppKitKit.stack([], spacing: 8)
    private var cardWidth: NSLayoutConstraint?
    private var cardHeight: NSLayoutConstraint?

    public init(update: DesktopUpdate, inCall: Bool, close: @escaping () -> Void,
                restart: @escaping () -> Void, download: @escaping () -> Void,
                report: ((String) async -> String?)? = nil) {
        self.update = update; self.inCall = inCall; self.close = close
        self.restart = restart; self.download = download; self.report = report
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = NSColor(CaperTheme.blackout).withAlphaComponent(0.86).cgColor
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        setAccessibilityLabel("Update review")
        setAccessibilityIdentifier("desktop-update-review")
        setAccessibilityModal(true)
        build()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    public override var acceptsFirstResponder: Bool { true }
    public override func cancelOperation(_ sender: Any?) { close() }
    public override func mouseDown(with event: NSEvent) {
        // The scrim closes; clicks on the card stay on the card.
        if !card.frame.contains(convert(event.locationInWindow, from: nil)) { close() }
    }
    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard window != nil else { return }
        alphaValue = 0
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.18
            animator().alphaValue = 1
        }
    }
    public override func layout() {
        super.layout()
        cardWidth?.constant = min(560, max(0, bounds.width - 32))
        cardHeight?.constant = min(660, max(0, bounds.height - 32))
    }

    private func build() {
        card.wantsLayer = true
        card.layer?.backgroundColor = AppKitKit.surface.cgColor
        card.layer?.cornerRadius = 8
        card.layer?.borderColor = AppKitKit.border.cgColor
        card.layer?.borderWidth = 1
        card.translatesAutoresizingMaskIntoConstraints = false
        addSubview(card)

        // The face, the title, then "Version 0.1.82 · October 18, 2026"; the version opens its commit.
        let version = NSStackView(); version.orientation = .horizontal; version.spacing = 0
        if let url = update.commitURL {
            version.addArrangedSubview(AppKitKitLink("Version \(update.version)", url: url))
        } else {
            version.addArrangedSubview(AppKitKit.label("Version \(update.version)", size: 13, color: AppKitKit.muted, wraps: false))
        }
        if let date = update.releasedLabel {
            version.addArrangedSubview(AppKitKit.label(" · \(date)", size: 13, color: AppKitKit.muted, wraps: false))
        }
        let face = AppKitKit.image("caper-face", size: 36)
        let heading = AppKitKit.stack([face, AppKitKit.label("A fresh Caper is ready", size: 20, weight: .bold, wraps: false), version], spacing: 2)
        heading.setCustomSpacing(10, after: face)
        let dismiss = AppKitKitButton("✕", kind: .plain) { [weak self] in self?.close() }
        dismiss.setAccessibilityLabel("Close update review")
        let header = NSView(); header.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(heading); header.addSubview(dismiss)
        NSLayoutConstraint.activate([
            heading.leadingAnchor.constraint(equalTo: header.leadingAnchor), heading.topAnchor.constraint(equalTo: header.topAnchor),
            heading.bottomAnchor.constraint(equalTo: header.bottomAnchor),
            dismiss.trailingAnchor.constraint(equalTo: header.trailingAnchor), dismiss.topAnchor.constraint(equalTo: header.topAnchor),
            heading.trailingAnchor.constraint(lessThanOrEqualTo: dismiss.leadingAnchor, constant: -12),
        ])

        let sectionLabel = AppKitKit.label("WHAT’S NEW", size: 11, weight: .bold, color: AppKitKit.muted, wraps: false)
        sectionLabel.attributedStringValue = NSAttributedString(string: "WHAT’S NEW", attributes: [
            .font: AppKitKit.font(11, weight: .bold), .foregroundColor: AppKitKit.muted, .kern: 0.8])
        sectionLabel.setAccessibilityRole(.staticText)
        let rule = AppKitKit.divider()
        let section = NSStackView(views: [sectionLabel, rule]); section.orientation = .horizontal; section.spacing = 10
        section.translatesAutoresizingMaskIntoConstraints = false

        let scroll = appKitKitScroll(history(), insets: NSEdgeInsets(top: 0, left: 0, bottom: 0, right: 12))

        var footer: [NSView] = [AppKitKit.divider()]
        if !update.canApply {
            footer.append(AppKitKit.label("This copy can’t update in place. Download the installer and drag Caper into a writable Applications folder.",
                                          size: 12, color: AppKitKit.muted))
        }
        if update.canApply, let size = update.downloadLabel {
            footer.append(AppKitKit.label(size, size: 12, color: AppKitKit.muted))
        }
        let later = AppKitKitButton("Later") { [weak self] in self?.close() }
        later.keyEquivalent = "\u{1b}"
        let install = AppKitKitButton(update.canApply ? (inCall ? "Restart and leave call" : "Restart to update") : "Download",
                                      kind: .primary) { [weak self] in
            guard let self else { return }
            self.update.canApply ? self.restart() : self.download()
        }
        install.setAccessibilityIdentifier("desktop-update-install")
        let spacer = NSView(); spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let actions = NSStackView(views: [later, spacer, install]); actions.orientation = .horizontal; actions.spacing = 10
        actions.translatesAutoresizingMaskIntoConstraints = false
        footer.append(actions)
        let bottom = AppKitKit.stack(footer, spacing: 16)
        for view in footer where !(view is NSTextField) { AppKitKit.fillWidth(view, of: bottom) }

        for view in [header, section, scroll, bottom] as [NSView] { card.addSubview(view) }
        let width = card.widthAnchor.constraint(equalToConstant: 560)
        let height = card.heightAnchor.constraint(equalToConstant: 660)
        cardWidth = width; cardHeight = height
        NSLayoutConstraint.activate([
            width, height,
            card.centerXAnchor.constraint(equalTo: centerXAnchor), card.centerYAnchor.constraint(equalTo: centerYAnchor),
            header.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 20),
            header.trailingAnchor.constraint(equalTo: card.trailingAnchor, constant: -20),
            header.topAnchor.constraint(equalTo: card.topAnchor, constant: 20),
            section.leadingAnchor.constraint(equalTo: header.leadingAnchor),
            section.trailingAnchor.constraint(equalTo: header.trailingAnchor),
            section.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 16),
            scroll.leadingAnchor.constraint(equalTo: header.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: header.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: section.bottomAnchor, constant: 16),
            bottom.leadingAnchor.constraint(equalTo: header.leadingAnchor),
            bottom.trailingAnchor.constraint(equalTo: header.trailingAnchor),
            bottom.topAnchor.constraint(equalTo: scroll.bottomAnchor, constant: 16),
            bottom.bottomAnchor.constraint(equalTo: card.bottomAnchor, constant: -20),
        ])
    }

    /// The independently scrolling history: the last failure, then the highlights
    /// (or the plain notes and changelog when the highlights could not be read).
    private func history() -> NSStackView {
        let content = AppKitKit.stack([], spacing: 16)
        func add(_ view: NSView) { content.addArrangedSubview(view); AppKitKit.fillWidth(view, of: content) }
        if let error = update.lastError {
            // The installer link sits in the sentence; no separate footer link.
            var sentence: [NSView] = [AppKitKit.label(update.canApply ? "Try again using the button below, or"
                                                      : "Download the installer with the button below.", color: AppKitKit.muted, wraps: false)]
            if update.canApply {
                let installer = AppKitKitButton("use the installer.", kind: .plain) { [weak self] in self?.download() }
                sentence.append(installer)
            }
            let detail = NSStackView(views: sentence); detail.orientation = .horizontal; detail.spacing = 0
            let failure = AppKitKit.label(error, size: 12, color: AppKitKit.error)
            failure.isSelectable = true
            let block = AppKitKit.stack([AppKitKit.label("The last update didn’t finish", size: 14, weight: .bold, color: AppKitKit.error),
                                         detail, failure, reportArea], spacing: 8)
            for view in [failure, reportArea] as [NSView] { AppKitKit.fillWidth(view, of: block) }
            add(block)
            renderReport(nil, error: error)
        }
        if !update.historyComplete {
            add(AppKitKit.label("Earlier release notes aren’t available.", size: 13, color: AppKitKit.muted))
        }
        if let highlights = update.highlights {
            if highlights.isEmpty { add(AppKitKit.label("Behind-the-scenes fixes this time.", color: AppKitKit.muted)) }
            for item in highlights { add(highlightRow(item)) }
        } else {
            let notes = AppKitKit.label(update.notes, color: AppKitKit.muted); notes.isSelectable = true
            add(notes)
            for entry in update.changelog {
                let label = AppKitKit.label(entry.notes, color: AppKitKit.muted); label.isSelectable = true
                add(label)
            }
        }
        return content
    }

    private enum ReportState: Equatable { case sending, sent, failed(String) }

    /// One click posts this error to the Caper team from your notes.
    private func renderReport(_ state: ReportState?, error: String) {
        reportArea.arrangedSubviews.forEach { $0.removeFromSuperview() }
        guard let report else { reportArea.isHidden = true; return }
        func button() -> NSView {
            let send = AppKitKitButton("Send this to the Caper team") { [weak self] in
                self?.renderReport(.sending, error: error)
                Task { @MainActor [weak self] in
                    let failure = await report(error)
                    self?.renderReport(failure.map { .failed($0) } ?? .sent, error: error)
                }
            }
            send.setAccessibilityIdentifier("desktop-update-report")
            return send
        }
        switch state {
        case .sent?:
            reportArea.addArrangedSubview(AppKitKit.label("Sent. The Caper team will reply in your notes.", size: 12, color: AppKitKit.muted))
        case .sending?:
            let sending = AppKitKitButton("Sending to the Caper team…") {}
            sending.isEnabled = false
            reportArea.addArrangedSubview(sending)
        case .failed(let message)?:
            reportArea.addArrangedSubview(AppKitKit.label(message, size: 12, color: AppKitKit.error))
            reportArea.addArrangedSubview(button())
        case nil:
            reportArea.addArrangedSubview(button())
        }
    }

    /// One release highlight: a bullet, a bold title and a muted description.
    private func highlightRow(_ item: DesktopUpdate.Highlight) -> NSView {
        let bullet = NSView(); bullet.wantsLayer = true
        bullet.layer?.backgroundColor = AppKitKit.muted.cgColor; bullet.layer?.cornerRadius = 2.5
        bullet.translatesAutoresizingMaskIntoConstraints = false
        let title = AppKitKit.label(item.title, size: 15, weight: .bold); title.isSelectable = true
        let body = AppKitKit.label(item.body, size: 14, color: AppKitKit.muted); body.isSelectable = true
        let paragraph = NSMutableParagraphStyle(); paragraph.lineSpacing = 3
        body.attributedStringValue = NSAttributedString(string: item.body, attributes: [
            .font: AppKitKit.font(14), .foregroundColor: AppKitKit.muted, .paragraphStyle: paragraph])
        let text = AppKitKit.stack([title, body], spacing: 2)
        AppKitKit.fillWidth(title, of: text); AppKitKit.fillWidth(body, of: text)
        let row = NSView(); row.translatesAutoresizingMaskIntoConstraints = false
        row.addSubview(bullet); row.addSubview(text)
        row.setAccessibilityElement(true)
        row.setAccessibilityRole(.staticText)
        row.setAccessibilityLabel("\(item.title), \(item.body)")
        NSLayoutConstraint.activate([
            bullet.widthAnchor.constraint(equalToConstant: 5), bullet.heightAnchor.constraint(equalToConstant: 5),
            bullet.leadingAnchor.constraint(equalTo: row.leadingAnchor),
            bullet.centerYAnchor.constraint(equalTo: title.centerYAnchor),
            text.leadingAnchor.constraint(equalTo: bullet.trailingAnchor, constant: 10),
            text.trailingAnchor.constraint(equalTo: row.trailingAnchor),
            text.topAnchor.constraint(equalTo: row.topAnchor), text.bottomAnchor.constraint(equalTo: row.bottomAnchor),
        ])
        return row
    }
}
#endif
