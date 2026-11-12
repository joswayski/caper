#if os(macOS)
import AppKit

/// An exact-username field with its suggestion list inline below it, so the
/// dialog never clips the list. As in the composer: at most six people, four
/// rows visible, Up and Down move the highlight, Return picks it, and Escape
/// closes the list (without closing the dialog). The list stays closed for a
/// picked username until the text changes.
@MainActor final class AppKitPeopleField: NSView, NSTextFieldDelegate {
    let field = NSTextField()
    private let list = NSStackView()
    private let listScroll = NSScrollView()
    private var listHeight: NSLayoutConstraint!
    private var rows: [AppKitPeopleRow] = []
    private var state = PeopleSuggestionState()
    private var choices: [MentionCandidate] = []
    private let normalize: (String) -> String
    /// The text the field shows; edits pass through `normalize`.
    private(set) var text = ""
    var candidates: [MentionCandidate] = [] { didSet { refreshList() } }
    /// Pending forms keep focus in the field but ignore edits and hide the list.
    var editable = true { didSet { refreshList() } }
    var changed: ((String) -> Void)?
    /// Return while no list is open.
    var submit: (() -> Void)?

    init(placeholder: String, identifier: String, listIdentifier: String, normalize: @escaping (String) -> String) {
        self.normalize = normalize
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        field.translatesAutoresizingMaskIntoConstraints = false
        field.placeholderString = placeholder
        field.font = AppKitKit.font(14)
        field.bezelStyle = .roundedBezel
        field.isAutomaticTextCompletionEnabled = false
        field.delegate = self
        field.setAccessibilityIdentifier(identifier)
        field.setAccessibilityLabel(placeholder)

        list.orientation = .vertical; list.spacing = 0; list.alignment = .leading
        list.translatesAutoresizingMaskIntoConstraints = false
        listScroll.drawsBackground = true
        listScroll.backgroundColor = AppKitKit.surface
        listScroll.hasVerticalScroller = true; listScroll.autohidesScrollers = true
        listScroll.translatesAutoresizingMaskIntoConstraints = false
        listScroll.wantsLayer = true
        listScroll.layer?.cornerRadius = 8; listScroll.layer?.borderWidth = 1
        listScroll.layer?.borderColor = AppKitKit.border.cgColor
        let document = AppKitKitFlippedView(); document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(list)
        listScroll.documentView = document
        listScroll.setAccessibilityIdentifier(listIdentifier)
        listScroll.isHidden = true
        addSubview(field); addSubview(listScroll)
        listHeight = listScroll.heightAnchor.constraint(equalToConstant: 0)
        let listTop = listScroll.topAnchor.constraint(equalTo: field.bottomAnchor, constant: 6)
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: leadingAnchor), field.trailingAnchor.constraint(equalTo: trailingAnchor),
            field.topAnchor.constraint(equalTo: topAnchor), field.heightAnchor.constraint(greaterThanOrEqualToConstant: 28),
            listScroll.leadingAnchor.constraint(equalTo: leadingAnchor), listScroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            listTop, listHeight, listScroll.bottomAnchor.constraint(equalTo: bottomAnchor),
            document.leadingAnchor.constraint(equalTo: listScroll.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: listScroll.contentView.trailingAnchor),
            document.topAnchor.constraint(equalTo: listScroll.contentView.topAnchor),
            list.leadingAnchor.constraint(equalTo: document.leadingAnchor), list.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            list.topAnchor.constraint(equalTo: document.topAnchor), list.bottomAnchor.constraint(equalTo: document.bottomAnchor),
        ])
        self.listTop = listTop
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    private var listTop: NSLayoutConstraint!

    /// Sets the text without treating it as an edit, e.g. a fresh form.
    func setText(_ value: String) {
        text = value; field.stringValue = value
        state.textChanged(to: value); refreshList()
    }

    func controlTextDidChange(_ notification: Notification) {
        guard editable else { field.stringValue = text; return }
        let normalized = normalize(field.stringValue)
        if field.stringValue != normalized { field.stringValue = normalized }
        guard normalized != text else { return }
        text = normalized
        state.textChanged(to: normalized)
        refreshList()
        changed?(normalized)
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.moveUp(_:)), #selector(NSResponder.moveDown(_:)):
            guard !choices.isEmpty else { return false }
            state.move(down: selector == #selector(NSResponder.moveDown(_:)), count: choices.count)
            showHighlight(); return true
        case #selector(NSResponder.cancelOperation(_:)):
            guard !choices.isEmpty else { return false }
            state.close(for: text); refreshList(); return true
        case #selector(NSResponder.insertNewline(_:)):
            // Return only picks a suggestion; the dialog's button acts.
            if let choice = state.highlightedChoice(in: choices) { pick(choice) } else { submit?() }
            return true
        default: return false
        }
    }

    private func pick(_ candidate: MentionCandidate) {
        guard editable else { return }
        text = candidate.username; field.stringValue = text
        state.close(for: text)
        refreshList()
        changed?(text)
    }

    private func refreshList() {
        let next = state.choices(text: text, candidates: candidates, enabled: editable)
        guard next != choices || rows.count != next.count else { showHighlight(); return }
        choices = next
        rows.forEach { $0.removeFromSuperview() }
        rows = next.map { candidate in AppKitPeopleRow(candidate: candidate) { [weak self] in self?.pick(candidate) } }
        for row in rows { list.addArrangedSubview(row); row.widthAnchor.constraint(equalTo: list.widthAnchor).isActive = true }
        listScroll.isHidden = rows.isEmpty
        listHeight.constant = rows.isEmpty ? 0 : min(CGFloat(rows.count) * 44, 176)
        listTop.constant = rows.isEmpty ? 0 : 6
        showHighlight()
    }

    private func showHighlight() {
        let index = state.highlightedIndex(count: rows.count)
        for (offset, row) in rows.enumerated() { row.current = offset == index }
        if rows.indices.contains(index) { rows[index].scrollToVisible(rows[index].bounds) }
    }
}

/// A person in the suggestion list: avatar, display name and @username.
@MainActor private final class AppKitPeopleRow: NSButton {
    private let handler: () -> Void
    var current = false { didSet { layer?.backgroundColor = (current ? AppKitKit.terracotta.withAlphaComponent(0.32) : .clear).cgColor } }

    init(candidate: MentionCandidate, action: @escaping () -> Void) {
        handler = action
        super.init(frame: .zero)
        title = ""; isBordered = false; bezelStyle = .regularSquare
        wantsLayer = true; focusRingType = .none
        target = self; self.action = #selector(fire)
        translatesAutoresizingMaskIntoConstraints = false
        setAccessibilityLabel(candidate.accessibilityLabel)
        setAccessibilityIdentifier("person-suggestion-\(candidate.username.lowercased())")
        let avatar = AppKitAvatar(); avatar.update(candidate)
        avatar.translatesAutoresizingMaskIntoConstraints = false
        let name = AppKitKit.label(candidate.displayName, size: 13, weight: .medium, wraps: false)
        let username = AppKitKit.label("@\(candidate.username)\(candidate.invited ? " · Already invited" : "")", size: 11,
                                       color: AppKitKit.muted, wraps: false)
        let text = AppKitKit.stack([name, username], spacing: 1)
        for view in [avatar, name, username, text] as [NSView] { view.setAccessibilityElement(false) }
        addSubview(avatar); addSubview(text)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 44),
            avatar.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10), avatar.centerYAnchor.constraint(equalTo: centerYAnchor),
            avatar.widthAnchor.constraint(equalToConstant: 28), avatar.heightAnchor.constraint(equalToConstant: 28),
            text.leadingAnchor.constraint(equalTo: avatar.trailingAnchor, constant: 10),
            text.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -10),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    @objc private func fire() { handler() }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .pointingHand) }
}

/// Starts a DM by exact username: the first press finds the person, the next
/// opens the conversation. Picking a suggestion only fills the field.
@MainActor final class AppKitNewDirectMessageDialog: NSView, AppKitDialogContent {
    private let model: AppModel
    private let close: () -> Void
    private let header: AppKitDialogHeader
    private let people = AppKitPeopleField(placeholder: "Exact username", identifier: "dm-username",
                                           listIdentifier: "dm-username-suggestions", normalize: ProfileValidation.normalizeUsername)
    private let status = AppKitKit.label("", size: 12)
    private var submitButton: AppKitKitButton!
    private let observation = AppKitObservation()
    private var resolved: Person?
    private var error: String?
    private var pending = false
    var dismissDisabled: Bool { pending }

    private var username: String { people.text }
    private var candidates: [MentionCandidate] { PeopleSuggestions.directMessage(people: model.people, accountID: model.account?.id) }
    private var recipientName: String? { resolved?.displayName ?? candidates.first(where: { $0.username == username })?.displayName }
    private var validUsername: Bool { username.range(of: "^[a-z0-9_]{3,32}$", options: .regularExpression) != nil }

    init(model: AppModel, close: @escaping () -> Void) {
        self.model = model; self.close = close
        var dismiss: () -> Void = {}
        header = AppKitDialogHeader(title: "New direct message", detail: "Enter an account’s exact username.") { dismiss() }
        super.init(frame: .zero)
        dismiss = { [weak self] in if self?.pending == false { close() } }
        wantsLayer = true
        layer?.backgroundColor = AppKitKit.surface.cgColor
        submitButton = AppKitKitButton("Find person", kind: .primary) { [weak self] in self?.start() }
        submitButton.setAccessibilityIdentifier("dm-create-submit")
        submitButton.widthAnchor.constraint(equalToConstant: 180).isActive = true
        people.changed = { [weak self] _ in self?.error = nil; self?.resolved = nil; self?.render() }
        status.translatesAutoresizingMaskIntoConstraints = false
        // The status line keeps its space so the button never jumps.
        let statusBox = NSView(); statusBox.translatesAutoresizingMaskIntoConstraints = false
        statusBox.addSubview(status)
        let spacer = NSView(); spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        let buttons = NSStackView(views: [spacer, submitButton]); buttons.orientation = .horizontal
        let body = AppKitKit.stack([people, statusBox, buttons], spacing: 18)
        for view in [people, statusBox, buttons] as [NSView] { AppKitKit.fillWidth(view, of: body) }
        addSubview(header); addSubview(body)
        NSLayoutConstraint.activate([
            header.leadingAnchor.constraint(equalTo: leadingAnchor), header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.topAnchor.constraint(equalTo: topAnchor),
            body.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 22), body.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -22),
            body.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 18), body.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -22),
            statusBox.heightAnchor.constraint(greaterThanOrEqualToConstant: 48),
            status.leadingAnchor.constraint(equalTo: statusBox.leadingAnchor), status.trailingAnchor.constraint(equalTo: statusBox.trailingAnchor),
            status.topAnchor.constraint(equalTo: statusBox.topAnchor), status.bottomAnchor.constraint(lessThanOrEqualTo: statusBox.bottomAnchor),
        ])
        observation.start(reading: { [model] in model.people }) { [weak self] _ in
            guard let self else { return }
            self.people.candidates = self.candidates; self.render()
        }
        Task { [model] in await model.refreshPeople() }
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        DispatchQueue.main.async { [weak self] in
            guard let self, let window = self.window else { return }
            window.makeFirstResponder(self.people.field)
        }
    }

    private func render() {
        let found = recipientName != nil
        let label = pending ? (found ? "Opening…" : "Finding…") : (found ? "Open conversation" : "Find person")
        submitButton.title = label
        submitButton.setAccessibilityLabel(label)
        submitButton.isEnabled = !pending && validUsername
        header.close.isEnabled = !pending
        people.editable = !pending
        if let error {
            status.stringValue = error; status.textColor = AppKitKit.error
            status.setAccessibilityIdentifier("dm-create-error"); status.isHidden = false
        } else if let recipientName {
            status.stringValue = "\(recipientName) (@\(resolved?.username ?? username))"; status.textColor = AppKitKit.muted
            status.setAccessibilityIdentifier(nil); status.isHidden = false
        } else { status.isHidden = true }
    }

    private func start() {
        guard !pending, validUsername else { return }
        let submitted = resolved?.username ?? username
        let opening = recipientName != nil
        pending = true; error = nil; render()
        Task { [weak self, model, close] in
            var failure: String?, person: Person?, opened = false
            do {
                if opening { opened = try await model.createDirectMessage(username: submitted); if !opened { failure = "Couldn’t open that conversation." } }
                else { person = try await model.lookupDirectPerson(username: submitted) }
            } catch { failure = FriendlyError.message(for: error) }
            guard let self else { return }
            self.pending = false
            if opened { close(); return }
            if !opening { self.resolved = person }
            self.error = failure
            self.render()
        }
    }
}
#endif
