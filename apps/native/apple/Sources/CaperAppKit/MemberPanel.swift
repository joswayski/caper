import AppKit
import CaperCore

@MainActor final class MemberPanel: NativeView, NSSearchFieldDelegate {
    private let model: AppModel
    private let observation = NativeObservation()
    private let title = NativeStyle.label("Members", size: 12, bold: true)
    private let count = NativeStyle.label("", size: 10, bold: true)
    private let search = NSSearchField()
    private let scroll = NSScrollView()
    private let document = NativeView()
    private let rule = NativeView(NativeStyle.border)
    private let leftRule = NativeView(NativeStyle.border)
    private lazy var searchButton = NativeButton.icon("Search members", symbol: "magnifyingglass") { [weak self] in
        self?.setSearching(true)
    }
    private lazy var clearButton = NativeButton.icon("Clear and close member search", symbol: "xmark") { [weak self] in
        self?.setSearching(false)
    }
    private lazy var closeButton = NativeButton.icon("Close member list", symbol: "xmark") { [weak self] in self?.close?() }
    var close: (() -> Void)?
    var openProfile: ((Member, NSView) -> Void)?
    private var searching = false
    private var spaceID: String?
    private var entries: [Entry] = []
    private var realized: [Int: NSView] = [:]
    private var rendering = false

    private struct Entry {
        let y: CGFloat
        let height: CGFloat
        let text: String
        var member: Member? = nil
        var status: PresenceStatus = .unknown
        var quiet = false
        var raised = false
    }

    init(model: AppModel) {
        self.model = model
        super.init(NativeStyle.sidebar)
        [title, count, searchButton, closeButton, search, clearButton, scroll, rule, leftRule].forEach { addSubview($0) }
        title.textColor = NativeStyle.muted; count.textColor = NativeStyle.muted
        count.setAccessibilityIdentifier("member-count")
        search.placeholderString = "Name or username"
        search.font = NativeStyle.font(12)
        search.setAccessibilityLabel("Name or username")
        search.delegate = self
        search.sendsSearchStringImmediately = true
        search.isHidden = true; clearButton.isHidden = true
        scroll.documentView = document
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true; scroll.autohidesScrollers = true
        scroll.setAccessibilityLabel("Members")
        scroll.setAccessibilityIdentifier("member-list")
        document.setAccessibilityElement(false)
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(scrolled), name: NSView.boundsDidChangeNotification, object: scroll.contentView)
        observation.start { [weak self] in self?.updateMembers() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { NotificationCenter.default.removeObserver(self) }

    func setVisible(_ visible: Bool) {
        guard isHidden == visible else { return }
        layer?.removeAnimation(forKey: "entrance")
        isHidden = !visible
        if !visible {
            setSearching(false)
            discardRows()
        } else {
            needsLayout = true
            if !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
                let opacity = CABasicAnimation(keyPath: "opacity")
                opacity.fromValue = 0; opacity.toValue = 1
                let offset = CABasicAnimation(keyPath: "transform.translation.x")
                offset.fromValue = 12; offset.toValue = 0
                let entrance = CAAnimationGroup()
                entrance.animations = [opacity, offset]
                // shared/design.css --motion-base and --ease-out, as in CaperMotion.panel.
                entrance.duration = 0.2
                entrance.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
                layer?.add(entrance, forKey: "entrance")
            }
        }
    }

    override func layout() {
        super.layout()
        let width = bounds.width
        let titleWidth = ceil(title.cell?.cellSize.width ?? 60)
        title.frame = NSRect(x: 12, y: 17, width: titleWidth, height: 18)
        count.frame = NSRect(x: title.frame.maxX + 4, y: 19, width: max(0, width - title.frame.maxX - 80), height: 16)
        searchButton.frame = NSRect(x: width - 72, y: 9, width: 28, height: 32)
        closeButton.frame = NSRect(x: width - 36, y: 9, width: 28, height: 32)
        rule.frame = NSRect(x: 0, y: 49, width: width, height: 1)
        leftRule.frame = NSRect(x: 0, y: 0, width: 1, height: bounds.height)
        search.frame = NSRect(x: 12, y: 58, width: max(0, width - 52), height: 26)
        clearButton.frame = NSRect(x: width - 36, y: 55, width: 28, height: 32)
        let top: CGFloat = searching ? 92 : 50
        scroll.frame = NSRect(x: 0, y: top, width: width, height: max(0, bounds.height - top))
        realize()
    }

    private func setSearching(_ value: Bool) {
        searching = value
        search.isHidden = !value; clearButton.isHidden = !value
        searchButton.isHidden = value || model.detail?.space.demo == true
        if !value {
            model.presence.query = ""; search.stringValue = ""
            scroll.contentView.scroll(to: .zero)
        }
        needsLayout = true
        layoutSubtreeIfNeeded()
        if value { window?.makeFirstResponder(search) }
        else if window?.firstResponder === search.currentEditor() { window?.makeFirstResponder(nil) }
    }

    func controlTextDidChange(_ notification: Notification) {
        scroll.contentView.scroll(to: .zero)
        model.presence.query = search.stringValue
    }
    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        guard commandSelector == #selector(NSResponder.cancelOperation(_:)) else { return false }
        setSearching(false)
        return true
    }

    private func updateMembers() {
        if spaceID != model.selectedSpaceID {
            spaceID = model.selectedSpaceID
            setSearching(false)
            scroll.contentView.scroll(to: .zero)
        }
        let presence = model.presence
        let demo = model.detail?.space.demo == true
        count.stringValue = demo ? "" : "\(model.detail?.members.count ?? 0)"
        searchButton.isHidden = searching || demo
        if search.stringValue != presence.query {
            search.stringValue = presence.query
            scroll.contentView.scroll(to: .zero)
        }
        entries.removeAll(keepingCapacity: true)
        if demo { append("General is open to everyone. People in voice appear in the channel sidebar.", height: 70) }
        else if model.detail?.members.isEmpty != false { append("No members to show.", height: 32) }
        else if presence.loading { append("Loading members…", height: 32) }
        else {
            let visible = presence.visibleMembers
            if visible.isEmpty { append("No matching members.", height: 32) }
            if let groups = model.detail?.memberGroups(channelID: model.selectedChannelID) {
                let ids = Set(groups.inChannel.map(\.id))
                let top = visible.filter { ids.contains($0.id) }
                let rest = visible.filter { !ids.contains($0.id) }
                if !top.isEmpty {
                    let channel = model.detail?.channels.first { $0.id == model.selectedChannelID }?.name ?? ""
                    append("In #\(channel)  \(top.count)", raised: true)
                    appendSections(top, raised: true)
                }
                if !rest.isEmpty {
                    append("Others in this space  \(rest.count)")
                    appendSections(rest, quiet: true)
                }
            } else { appendSections(visible) }
        }
        discardRows()
        realize()
    }

    private func append(_ text: String, height: CGFloat = 28, member: Member? = nil,
                        status: PresenceStatus = .unknown, quiet: Bool = false, raised: Bool = false) {
        let y = entries.last.map { $0.y + $0.height } ?? 4
        entries.append(Entry(y: y, height: height, text: text, member: member, status: status, quiet: quiet, raised: raised))
    }

    private func appendSections(_ members: [Member], quiet: Bool = false, raised: Bool = false) {
        for section in [PresenceStatus.online, .offline, .unknown] {
            let rows = members.filter {
                let status = model.presence.status(for: $0)
                return section == .online ? status == .online || status == .idle : status == section
            }
            guard !rows.isEmpty else { continue }
            let label = section == .online ? "Online" : section == .offline ? "Offline" : "Status unavailable"
            append("\(label)  \(rows.count)", raised: raised)
            for snapshot in rows {
                let member = model.profiled(snapshot)
                append(member.displayName, height: 38, member: member, status: model.presence.status(for: member),
                    quiet: quiet, raised: raised)
            }
        }
    }

    private func discardRows() {
        realized.values.forEach { $0.removeFromSuperview() }
        realized.removeAll(keepingCapacity: true)
    }
    @objc private func scrolled() { realize() }
    private func realize() {
        guard !rendering, !isHidden else { return }
        rendering = true
        defer { rendering = false }
        let height = entries.last.map { $0.y + $0.height + 8 } ?? 0
        document.setFrameSize(NSSize(width: scroll.contentSize.width, height: max(scroll.contentSize.height, height)))
        let clip = scroll.contentView.bounds
        // Binary search offsets, then create only visible rows (including a
        // partially clipped boundary row). AX never sees the entire roster.
        var low = 0, high = entries.count
        while low < high {
            let mid = (low + high) / 2
            if entries[mid].y + entries[mid].height <= clip.minY { low = mid + 1 } else { high = mid }
        }
        var end = low
        while end < entries.count, entries[end].y < clip.maxY { end += 1 }
        let range = low..<end
        for index in Array(realized.keys) where !range.contains(index) { realized.removeValue(forKey: index)?.removeFromSuperview() }
        for index in range {
            let entry = entries[index]
            let row: NSView
            if let existing = realized[index] { row = existing }
            else {
                if let member = entry.member {
                    row = MemberRow(member: member, status: entry.status, quiet: entry.quiet, raised: entry.raised) { [weak self] view in
                        self?.openProfile?(member, view)
                    }
                } else {
                    let label = NativeStyle.label(entry.text, size: 11, bold: true)
                    label.textColor = NativeStyle.muted
                    label.wantsLayer = true
                    label.layer?.backgroundColor = (entry.raised ? NSColor(CaperTheme.sidebarRaised) : .clear).cgColor
                    row = label
                }
                realized[index] = row
                document.addSubview(row)
            }
            row.frame = NSRect(x: 12, y: entry.y, width: max(0, clip.width - 24), height: entry.height)
        }
    }
}

private final class MemberRow: NativeView {
    private let avatar = NativeAvatar()
    private let name: NSTextField
    private let badge = CALayer()
    private let open: (NSView) -> Void
    private lazy var action = NativeButton("") { [weak self] in
        guard let self else { return }; self.open(self)
    }

    init(member: Member, status: PresenceStatus, quiet: Bool, raised: Bool, open: @escaping (NSView) -> Void) {
        self.open = open
        name = NativeStyle.label(member.displayName, size: 12)
        let background = raised ? NSColor(CaperTheme.sidebarRaised) : NativeStyle.sidebar
        super.init(background)
        addSubview(avatar); addSubview(name); addSubview(action)
        action.layer?.backgroundColor = NSColor.clear.cgColor
        action.profileFeedback = true
        action.setAccessibilityIdentifier("profile-member-\(member.id)")
        action.setAccessibilityLabel("Open profile for \(member.displayName)")
        avatar.update(member)
        name.textColor = quiet && status != .offline ? NativeStyle.muted : NativeStyle.text
        alphaValue = status == .offline ? 0.6 : 1
        badge.backgroundColor = (status == .online ? NSColor(CaperTheme.green) : status == .idle
            ? NSColor(srgbRed: 0.72, green: 0.60, blue: 0.35, alpha: 1) : NativeStyle.border).cgColor
        badge.cornerRadius = 5; badge.borderWidth = 2; badge.borderColor = background.cgColor
        badge.zPosition = 1
        layer?.addSublayer(badge)
        setAccessibilityElement(true); setAccessibilityRole(.group)
        setAccessibilityIdentifier("member-\(member.id)")
        let label = status == .online ? "Online" : status == .idle ? "Idle" : status == .offline ? "Offline" : "Status unavailable"
        setAccessibilityLabel("\(member.displayName), \(label)")
        name.setAccessibilityElement(false)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func layout() {
        super.layout()
        avatar.frame = NSRect(x: 6, y: 4, width: 30, height: 30)
        name.frame = NSRect(x: 46, y: 10, width: max(0, bounds.width - 52), height: 18)
        action.frame = bounds
        CATransaction.begin(); CATransaction.setDisableActions(true)
        badge.frame = NSRect(x: 26, y: 24, width: 10, height: 10)
        CATransaction.commit()
    }
}
