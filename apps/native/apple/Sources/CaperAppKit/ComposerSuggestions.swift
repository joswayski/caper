import AppKit
import CaperCore

/// At most six shared candidates, with four rows visible just like SwiftUI.
/// This lives above the field, not inside its clipped scroll view, and never
/// takes the text view's first responder while navigating with arrows.
@MainActor final class ComposerSuggestions: NativeView {
    private let scroll = NSScrollView()
    private let document = NativeView()
    private var rows: [SuggestionRow] = []
    private(set) var match: ComposerAutocompleteMatch?
    private(set) var selected = 0
    var accept: ((Int) -> Void)?
    var height: CGFloat { min(CGFloat(match?.count ?? 0) * 44, 176) + 2 }

    init() {
        super.init(NSColor(CaperTheme.surface))
        layer?.borderWidth = 1; layer?.borderColor = NativeStyle.border.cgColor
        layer?.cornerRadius = 6; layer?.masksToBounds = true
        scroll.documentView = document
        scroll.hasVerticalScroller = true; scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        addSubview(scroll)
        isHidden = true
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(_ match: ComposerAutocompleteMatch?) {
        guard self.match != match else { return }
        let opening = self.match == nil && match != nil
        self.match = match; selected = 0
        rows.forEach { $0.removeFromSuperview() }; rows.removeAll()
        isHidden = match == nil
        guard let match else { layer?.removeAnimation(forKey: "entrance"); return }
        setAccessibilityIdentifier(match.isMention ? "mention-suggestions" : "emoji-suggestions")
        for index in 0..<match.count {
            let row = SuggestionRow(match: match, index: index) { [weak self] in self?.accept?(index) }
            rows.append(row); document.addSubview(row)
        }
        showSelection()
        needsLayout = true
        scroll.contentView.scroll(to: .zero)
        if opening, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            let opacity = CABasicAnimation(keyPath: "opacity")
            opacity.fromValue = 0; opacity.toValue = 1
            let offset = CABasicAnimation(keyPath: "transform.translation.y")
            offset.fromValue = 4; offset.toValue = 0
            let entrance = CAAnimationGroup()
            entrance.animations = [opacity, offset]
            // shared/design.css --motion-fast and --ease-out.
            entrance.duration = 0.14
            entrance.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
            layer?.add(entrance, forKey: "entrance")
        }
    }

    func move(down: Bool) {
        guard !rows.isEmpty else { return }
        selected = (selected + (down ? 1 : rows.count - 1)) % rows.count
        showSelection()
        rows[selected].scrollToVisible(rows[selected].bounds)
    }

    private func showSelection() {
        for (index, row) in rows.enumerated() { row.select(index == selected) }
    }

    override func layout() {
        super.layout()
        scroll.frame = bounds.insetBy(dx: 1, dy: 1)
        document.setFrameSize(NSSize(width: scroll.contentSize.width, height: CGFloat(rows.count) * 44))
        for (index, row) in rows.enumerated() {
            row.frame = NSRect(x: 0, y: CGFloat(index) * 44, width: document.bounds.width, height: 44)
        }
    }
}

@MainActor private final class SuggestionRow: NativeView {
    private let title = NativeStyle.label("", size: 13)
    private let subtitle = NativeStyle.label("", size: 11)
    private let icon = NSImageView()
    private var avatar: NativeAvatar?
    private let button: NativeButton

    init(match: ComposerAutocompleteMatch, index: Int, accept: @escaping () -> Void) {
        button = NativeButton("", action: accept)
        super.init()
        button.layer?.backgroundColor = NSColor.clear.cgColor
        title.setAccessibilityElement(false); subtitle.setAccessibilityElement(false); icon.setAccessibilityElement(false)
        subtitle.textColor = NativeStyle.muted
        switch match {
        case let .mention(match):
            let candidate = match.choices[index]
            let person = candidate.kind == .member
            title.stringValue = person ? candidate.displayName : "@\(candidate.username)"
            subtitle.stringValue = person ? "@\(candidate.username)" : candidate.displayName
            button.setAccessibilityIdentifier("mention-suggestion-\(candidate.username.lowercased())")
            button.setAccessibilityLabel(candidate.accessibilityLabel)
            if person {
                let avatar = NativeAvatar(); avatar.update(candidate); addSubview(avatar); self.avatar = avatar
            } else {
                icon.image = NSImage(systemSymbolName: "at", accessibilityDescription: nil)
                icon.contentTintColor = NativeStyle.muted
            }
        case let .emoji(match):
            let entry = match.choices[index]
            title.stringValue = ":\(entry.name.replacingOccurrences(of: " ", with: "_")):"
            subtitle.isHidden = true
            if let image = NativePresentation.emojiImage(for: entry) {
                icon.image = NSImage(cgImage: image, size: NSSize(width: 24, height: 24))
            }
            button.setAccessibilityIdentifier("emoji-suggestion-\(entry.id)")
            button.setAccessibilityLabel("\(entry.name), emoji")
        }
        icon.imageScaling = .scaleProportionallyUpOrDown
        [title, subtitle, icon, button].forEach { addSubview($0) }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func select(_ selected: Bool) {
        layer?.backgroundColor = (selected ? NativeStyle.accent.withAlphaComponent(0.32) : .clear).cgColor
        button.setAccessibilitySelected(selected)
    }

    override func layout() {
        super.layout()
        avatar?.frame = NSRect(x: 10, y: 8, width: 28, height: 28)
        icon.frame = NSRect(x: 12, y: 10, width: 24, height: 24)
        title.frame = NSRect(x: 48, y: subtitle.isHidden ? 13 : 5, width: max(0, bounds.width - 58), height: 18)
        subtitle.frame = NSRect(x: 48, y: 23, width: max(0, bounds.width - 58), height: 16)
        button.frame = bounds
    }
}
