#if os(macOS)
import AppKit

/// Content of a native workspace dialog. While `dismissDisabled` (a request is
/// running), Escape and the backdrop keep the dialog open.
@MainActor protocol AppKitDialogContent: NSView {
    var dismissDisabled: Bool { get }
}

/// The workspace dialog frame: a dimmed backdrop and a card 10% from the top,
/// as wide as the dialog asks and as tall as its content up to 680pt.
@MainActor final class AppKitDialogFrame: NSView {
    let content: AppKitDialogContent
    private let requestClose: () -> Void
    private let card = NSView()
    private let width: CGFloat
    private var top: NSLayoutConstraint?
    private var cardWidth: NSLayoutConstraint?
    private var maxHeight: NSLayoutConstraint?

    init(content: AppKitDialogContent, width: CGFloat, requestClose: @escaping () -> Void) {
        self.content = content; self.width = width; self.requestClose = requestClose
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.withAlphaComponent(0.55).cgColor
        card.wantsLayer = true
        card.layer?.backgroundColor = AppKitKit.surface.cgColor
        card.layer?.cornerRadius = 8
        card.layer?.borderColor = AppKitKit.border.cgColor
        card.layer?.borderWidth = 1
        card.layer?.masksToBounds = true
        card.translatesAutoresizingMaskIntoConstraints = false
        card.setAccessibilityElement(true)
        card.setAccessibilityRole(.group)
        card.setAccessibilityModal(true)
        content.translatesAutoresizingMaskIntoConstraints = false
        addSubview(card); card.addSubview(content)
        let top = card.topAnchor.constraint(equalTo: topAnchor, constant: 16)
        let cardWidth = card.widthAnchor.constraint(equalToConstant: width)
        let maxHeight = card.heightAnchor.constraint(lessThanOrEqualToConstant: 680)
        self.top = top; self.cardWidth = cardWidth; self.maxHeight = maxHeight
        NSLayoutConstraint.activate([
            top, cardWidth, maxHeight, card.centerXAnchor.constraint(equalTo: centerXAnchor),
            content.leadingAnchor.constraint(equalTo: card.leadingAnchor), content.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            content.topAnchor.constraint(equalTo: card.topAnchor), content.bottomAnchor.constraint(equalTo: card.bottomAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override func cancelOperation(_ sender: Any?) { if !content.dismissDisabled { requestClose() } }
    override func mouseDown(with event: NSEvent) {
        guard !card.frame.contains(convert(event.locationInWindow, from: nil)) else { return }
        if !content.dismissDisabled { requestClose() }
    }
    override func layout() {
        let offset = max(16, bounds.height * 0.1)
        top?.constant = offset
        cardWidth?.constant = min(width, max(1, bounds.width - 32))
        maxHeight?.constant = min(680, max(1, bounds.height - offset - 16))
        super.layout()
    }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard window != nil else { return }
        alphaValue = 0
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.15
            animator().alphaValue = 1
        }
    }
}

/// A dialog header: the title, an optional muted detail, and a close button.
@MainActor final class AppKitDialogHeader: NSView {
    let close: AppKitKitButton

    init(title: String, detail: String? = nil, closeLabel: String = "Close", close action: @escaping () -> Void) {
        close = AppKitKitButton("✕", kind: .plain, action: action)
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        close.setAccessibilityLabel(closeLabel)
        var rows: [NSView] = [AppKitKit.label(title, size: 20, weight: .bold)]
        if let detail { rows.append(AppKitKit.label(detail, size: 12, color: AppKitKit.muted)) }
        let text = AppKitKit.stack(rows, spacing: 6)
        for row in rows { AppKitKit.fillWidth(row, of: text) }
        let line = AppKitKit.divider()
        addSubview(text); addSubview(close); addSubview(line)
        NSLayoutConstraint.activate([
            text.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 22),
            text.topAnchor.constraint(equalTo: topAnchor, constant: 22),
            text.trailingAnchor.constraint(equalTo: close.leadingAnchor, constant: -12),
            text.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -22),
            close.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -22),
            close.topAnchor.constraint(equalTo: topAnchor, constant: 22),
            close.heightAnchor.constraint(equalToConstant: 27),
            line.leadingAnchor.constraint(equalTo: leadingAnchor), line.trailingAnchor.constraint(equalTo: trailingAnchor),
            line.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}

/// Confirms a destructive action, e.g. "Leave #design?". While it runs the
/// action reads its pending label and nothing closes the dialog; a failure
/// shows the error and lets the person retry or cancel.
@MainActor final class AppKitConfirmationDialog: NSView, AppKitDialogContent {
    private let action: String
    private let pendingLabel: String
    private let close: () -> Void
    private let perform: () async throws -> Void
    private let header: AppKitDialogHeader
    private let error = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var cancel: AppKitKitButton!
    private var confirm: AppKitKitButton!
    private(set) var dismissDisabled = false

    init(title: String, detail: String, action: String, pendingLabel: String? = nil,
         close: @escaping () -> Void, perform: @escaping () async throws -> Void) {
        self.action = action
        self.pendingLabel = pendingLabel ?? (action.hasPrefix("Delete") ? "Deleting…" : "Saving…")
        self.close = close; self.perform = perform
        var dismiss: () -> Void = {}
        header = AppKitDialogHeader(title: title, detail: detail) { dismiss() }
        super.init(frame: .zero)
        dismiss = { [weak self] in if self?.dismissDisabled == false { close() } }
        cancel = AppKitKitButton("Cancel") { [weak self] in if self?.dismissDisabled == false { close() } }
        cancel.keyEquivalent = "\u{1b}"
        confirm = AppKitKitButton(action, kind: .destructive) { [weak self] in self?.run() }
        confirm.setAccessibilityIdentifier("confirm-destructive-action")
        error.isHidden = true
        let spacer = NSView(); spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        let buttons = NSStackView(views: [spacer, cancel, confirm]); buttons.orientation = .horizontal; buttons.spacing = 8
        let body = AppKitKit.stack([error, buttons], spacing: 16)
        AppKitKit.fillWidth(error, of: body); AppKitKit.fillWidth(buttons, of: body)
        addSubview(header); addSubview(body)
        NSLayoutConstraint.activate([
            header.leadingAnchor.constraint(equalTo: leadingAnchor), header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.topAnchor.constraint(equalTo: topAnchor),
            body.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 22),
            body.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -22),
            body.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 22),
            body.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -22),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func setPending(_ pending: Bool) {
        dismissDisabled = pending
        confirm.title = pending ? pendingLabel : action
        confirm.isEnabled = !pending; cancel.isEnabled = !pending; header.close.isEnabled = !pending
    }

    private func run() {
        guard !dismissDisabled else { return }
        setPending(true)
        error.isHidden = true
        Task { [weak self, perform, close] in
            do {
                try await perform()
                close()
            } catch {
                self?.error.stringValue = FriendlyError.message(for: error)
                self?.error.isHidden = false
            }
            self?.setPending(false)
        }
    }
}
/// Answers an invitation: Decline or Accept, each showing its own progress.
/// Nothing closes the dialog while an answer is being sent.
@MainActor final class AppKitInvitationDialog: NSView, AppKitDialogContent {
    private let close: () -> Void
    private let header: AppKitDialogHeader
    private let error = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var decline: AppKitKitButton!
    private var accept: AppKitKitButton!
    private let labels: (decline: String, accept: String, declining: String, accepting: String)
    private(set) var dismissDisabled = false

    init(title: String, detail: String?, lines: [NSTextField], pendingLabels: Bool,
         close: @escaping () -> Void, decline: @escaping () async throws -> Void, accept: @escaping () async throws -> Void) {
        self.close = close
        labels = ("Decline", "Accept", pendingLabels ? "Declining…" : "Decline", pendingLabels ? "Accepting…" : "Accept")
        var dismiss: () -> Void = {}
        header = AppKitDialogHeader(title: title, detail: detail) { dismiss() }
        super.init(frame: .zero)
        dismiss = { [weak self] in if self?.dismissDisabled == false { close() } }
        self.decline = AppKitKitButton("Decline") { [weak self] in self?.run(declining: true, decline) }
        self.accept = AppKitKitButton("Accept", kind: .primary) { [weak self] in self?.run(declining: false, accept) }
        self.accept.widthAnchor.constraint(equalToConstant: 120).isActive = true
        self.decline.minimumHeight = 42
        error.isHidden = true
        let spacer = NSView(); spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        let buttons = NSStackView(views: [spacer, self.decline, self.accept]); buttons.orientation = .horizontal; buttons.spacing = 8
        let rows: [NSView] = lines + [error, buttons]
        let body = AppKitKit.stack(rows, spacing: 14)
        for row in rows { AppKitKit.fillWidth(row, of: body) }
        addSubview(header); addSubview(body)
        NSLayoutConstraint.activate([
            header.leadingAnchor.constraint(equalTo: leadingAnchor), header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.topAnchor.constraint(equalTo: topAnchor),
            body.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 22),
            body.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -22),
            body.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 18),
            body.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -22),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func run(declining: Bool, _ action: @escaping () async throws -> Void) {
        guard !dismissDisabled else { return }
        dismissDisabled = true; error.isHidden = true
        decline.isEnabled = false; accept.isEnabled = false; header.close.isEnabled = false
        (declining ? decline : accept)?.title = declining ? labels.declining : labels.accepting
        Task { [weak self, close] in
            do { try await action(); close() } catch {
                self?.error.stringValue = FriendlyError.message(for: error); self?.error.isHidden = false
            }
            guard let self else { return }
            self.dismissDisabled = false
            self.decline.title = self.labels.decline; self.accept.title = self.labels.accept
            self.decline.isEnabled = true; self.accept.isEnabled = true; self.header.close.isEnabled = true
        }
    }
}
#endif
