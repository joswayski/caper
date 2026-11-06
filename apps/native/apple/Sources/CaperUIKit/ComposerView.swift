import CaperCore
import SwiftUI
import UIKit

/// The conversation composer, as SwiftUI's phone layout draws it: one box
/// holding Attach (+), a native TextKit 2 UITextView, Emojis/GIFs/Stickers and
/// the microphone, with Send beside it and staged files and voice notices
/// above. The text view keeps dictation, marked text (Japanese, Chinese,
/// Korean IMEs), autocorrect and VoiceOver's value, caret and selection.
final class ComposerView: UIView, UITextViewDelegate {
    let textView = ComposerTextView(usingTextLayoutManager: true)
    private let placeholder = UIKitStyle.label(size: 14, color: UIKitStyle.muted)
    private let field = UIView()
    private let fieldRow = UIStackView()
    private lazy var attach = IconButton(icon: "plus", label: "Attach files", size: 18, side: 42) { [weak self] in self?.attachTapped?() }
    private lazy var emoji = IconButton(icon: "smile", label: "Emojis, GIFs and stickers", size: 18, side: 42) { [weak self] in self?.emojiTapped?() }
    private lazy var send = IconButton(icon: "send", label: "Send message", size: 16, side: 42) { [weak self] in self?.submit() }
    /// SwiftUI leaves: staged files and voice notices; the microphone; its recording bar.
    private let accessories = HostedView()
    private let microphone = HostedView()
    private let recordingBar = HostedView()
    private var height: NSLayoutConstraint!
    private let font = UIKitStyle.font(14)
    /// Text edits; the owner writes them to the model's draft.
    var changed: ((String) -> Void)?
    var submitted: (() -> Void)?
    var attachTapped: (() -> Void)?
    var emojiTapped: (() -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        field.translatesAutoresizingMaskIntoConstraints = false
        field.backgroundColor = UIKitStyle.composer
        field.layer.cornerRadius = 8
        field.layer.cornerCurve = .continuous
        field.layer.borderWidth = 1
        field.layer.borderColor = UIKitStyle.border.cgColor
        field.clipsToBounds = true
        addSubview(field)

        textView.translatesAutoresizingMaskIntoConstraints = false
        textView.backgroundColor = .clear
        textView.font = font
        textView.adjustsFontForContentSizeCategory = true
        textView.textColor = UIKitStyle.text
        textView.tintColor = UIKitStyle.terracottaBright
        textView.keyboardAppearance = .dark
        textView.textContainerInset = UIEdgeInsets(top: 12, left: 0, bottom: 12, right: 0)
        textView.textContainer.lineFragmentPadding = 5
        textView.showsHorizontalScrollIndicator = false
        textView.delegate = self
        textView.returnPressed = { [weak self] in self?.submit() }
        textView.accessibilityIdentifier = "message-composer"
        // The text view's own label carries the prompt.
        placeholder.accessibilityElementsHidden = true
        placeholder.lineBreakMode = .byTruncatingTail
        let text = UIView()
        text.translatesAutoresizingMaskIntoConstraints = false
        text.addSubview(textView)
        text.addSubview(placeholder)

        for button in [attach, emoji] {
            button.tintColor = UIKitStyle.muted
            button.setContentHuggingPriority(.required, for: .horizontal)
        }
        attach.accessibilityIdentifier = "attach-files-button"
        emoji.accessibilityIdentifier = "composer-emoji"
        microphone.setContentHuggingPriority(.required, for: .horizontal)
        microphone.clipsToBounds = false
        fieldRow.addArrangedSubview(attach)
        fieldRow.addArrangedSubview(text)
        fieldRow.addArrangedSubview(emoji)
        fieldRow.alignment = .bottom
        fieldRow.translatesAutoresizingMaskIntoConstraints = false
        field.addSubview(fieldRow)
        recordingBar.isHidden = true
        field.addSubview(recordingBar)
        // Outside the clipped box, so the held microphone's lock hint can rise above it.
        addSubview(microphone)

        send.layer.cornerRadius = 8
        send.layer.cornerCurve = .continuous
        send.layer.borderWidth = 1
        send.accessibilityIdentifier = "send-message"
        addSubview(send)
        addSubview(accessories)

        height = textView.heightAnchor.constraint(equalToConstant: 42)
        NSLayoutConstraint.activate([
            accessories.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            accessories.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
            accessories.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18),
            field.topAnchor.constraint(equalTo: accessories.bottomAnchor),
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
            field.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
            send.leadingAnchor.constraint(equalTo: field.trailingAnchor, constant: 8),
            send.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18),
            send.bottomAnchor.constraint(equalTo: field.bottomAnchor),
            fieldRow.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: 4),
            fieldRow.trailingAnchor.constraint(equalTo: microphone.leadingAnchor),
            fieldRow.topAnchor.constraint(equalTo: field.topAnchor),
            fieldRow.bottomAnchor.constraint(equalTo: field.bottomAnchor),
            microphone.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -2),
            microphone.bottomAnchor.constraint(equalTo: field.bottomAnchor),
            microphone.widthAnchor.constraint(equalToConstant: 36),
            microphone.heightAnchor.constraint(equalToConstant: 42),
            // The bar covers the field beside the microphone, which stays under the finger.
            recordingBar.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: 8),
            recordingBar.trailingAnchor.constraint(equalTo: microphone.leadingAnchor),
            recordingBar.bottomAnchor.constraint(equalTo: field.bottomAnchor),
            recordingBar.heightAnchor.constraint(equalToConstant: 42),
            textView.leadingAnchor.constraint(equalTo: text.leadingAnchor),
            textView.trailingAnchor.constraint(equalTo: text.trailingAnchor),
            textView.topAnchor.constraint(equalTo: text.topAnchor),
            textView.bottomAnchor.constraint(equalTo: text.bottomAnchor),
            height,
            placeholder.leadingAnchor.constraint(equalTo: textView.leadingAnchor, constant: 5),
            placeholder.topAnchor.constraint(equalTo: textView.topAnchor, constant: 12),
            placeholder.trailingAnchor.constraint(lessThanOrEqualTo: textView.trailingAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(prompt: String, enabled: Bool, canSend: Bool, canAttach: Bool, attachReason: String?, canPickExpressions: Bool) {
        placeholder.text = prompt
        textView.accessibilityLabel = prompt
        textView.isEditable = enabled
        send.isEnabled = enabled && canSend
        // Eggshell when there is something to send; neutral chrome otherwise.
        send.backgroundColor = send.isEnabled ? UIKitStyle.action : .clear
        send.tintColor = send.isEnabled ? UIKitStyle.blackout : UIKitStyle.muted
        send.layer.borderColor = (send.isEnabled ? UIKitStyle.action : UIKitStyle.border).cgColor
        attach.isEnabled = enabled && canAttach
        attach.accessibilityValue = attachReason
        emoji.isEnabled = enabled && canPickExpressions
    }

    /// Staged files and voice notices above the box, when there are any.
    func showAccessories<Content: View>(_ content: Content?) {
        accessories.show(content)
    }

    /// The microphone and, while it records, the bar over the field.
    func showVoice<Button: View, Bar: View>(button: Button?, bar: Bar, recording: Bool) {
        microphone.show(button)
        microphone.isHidden = button == nil
        recordingBar.show(recording ? bar : nil)
        recordingBar.isHidden = !recording
        fieldRow.alpha = recording ? 0 : 1
        fieldRow.accessibilityElementsHidden = recording
        fieldRow.isUserInteractionEnabled = !recording
    }

    /// The model's draft, written back only when it differs (a send clears it)
    /// and never while an input method is composing.
    func show(draft: String) {
        guard textView.markedTextRange == nil, textView.text != draft else { return }
        textView.text = draft
        refresh()
    }

    /// Puts text at the caret (an emoji, dictated words), as typing would.
    @discardableResult
    func insert(_ text: String, spaced: Bool = false) -> Bool {
        var insertion = text
        if spaced, let range = textView.selectedTextRange {
            let before = textView.text(in: textView.textRange(from: textView.beginningOfDocument, to: range.start) ?? range) ?? ""
            if let last = before.last, !last.isWhitespace { insertion = " " + insertion }
        }
        guard (textView.text + insertion).unicodeScalars.count <= 4000 else { return false }
        textView.insertText(insertion)
        return true
    }

    func textViewDidChange(_ textView: UITextView) {
        refresh()
        changed?(textView.text)
    }

    private func submit() {
        // Committing marked text first keeps an IME's in-progress word in the message.
        if textView.markedTextRange != nil { textView.unmarkText() }
        submitted?()
    }

    private func refresh() {
        placeholder.isHidden = !textView.text.isEmpty
        let lineHeight = font.lineHeight
        let fitting = textView.sizeThatFits(CGSize(width: textView.bounds.width > 0 ? textView.bounds.width : 240, height: .greatestFiniteMagnitude)).height
        // One line to six, then it scrolls.
        let maximum = 24 + lineHeight * 6
        let next = min(max(42, ceil(fitting)), maximum)
        textView.isScrollEnabled = fitting > maximum
        if height.constant != next { height.constant = next }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        refresh()
    }

    /// The lock hint draws above the microphone, outside this view's bounds.
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        if let hit = super.hitTest(point, with: event) { return hit }
        let inMicrophone = convert(point, to: microphone)
        return microphone.hitTest(inMicrophone, with: event)
    }
}

/// A hardware Return sends; Shift-Return, Option-Return and the software
/// keyboard's return add a line. Return during IME composition confirms the
/// candidate, as the input method expects.
final class ComposerTextView: UITextView {
    var returnPressed: (() -> Void)?

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        if let key = presses.first?.key, key.keyCode == .keyboardReturnOrEnter,
           key.modifierFlags.isDisjoint(with: [.shift, .alternate]), markedTextRange == nil, let returnPressed {
            returnPressed()
            return
        }
        super.pressesBegan(presses, with: event)
    }
}

/// A SwiftUI leaf inside UIKit layout, sized to its content. Hidden and
/// zero-height when there is nothing to show.
final class HostedView: UIView {
    private var content: (UIView & UIContentView)?
    private var collapsed: NSLayoutConstraint!

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        backgroundColor = .clear
        collapsed = heightAnchor.constraint(equalToConstant: 0)
        collapsed.isActive = true
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func show<Content: View>(_ view: Content?) {
        guard let view else {
            content?.removeFromSuperview()
            content = nil
            collapsed.isActive = true
            return
        }
        let configuration = UIHostingConfiguration { view }.margins(.all, 0).background(.clear)
        if let content {
            content.configuration = configuration
        } else {
            let made = configuration.makeContentView()
            made.translatesAutoresizingMaskIntoConstraints = false
            made.backgroundColor = .clear
            addSubview(made)
            NSLayoutConstraint.activate([
                made.topAnchor.constraint(equalTo: topAnchor),
                made.leadingAnchor.constraint(equalTo: leadingAnchor),
                made.trailingAnchor.constraint(equalTo: trailingAnchor),
                made.bottomAnchor.constraint(equalTo: bottomAnchor),
            ])
            content = made
        }
        collapsed.isActive = false
        content?.invalidateIntrinsicContentSize()
    }
}

/// "Maya is typing…" above the composer, after three bouncing dots like web's
/// keyframes: up and back over the first 60% of a 1.2 s cycle, each dot 0.15 s
/// behind the last. Core Animation runs it off the main thread; Reduce Motion
/// keeps the dots still. Collapses to nothing when nobody is typing.
final class TypingIndicatorView: UIView {
    private let label = UIKitStyle.label(size: 11, color: UIKitStyle.muted)
    private let dots = (0..<3).map { _ in UIView() }
    private var visible: NSLayoutConstraint!

    override init(frame: CGRect) {
        super.init(frame: frame)
        translatesAutoresizingMaskIntoConstraints = false
        clipsToBounds = true
        let dotRow = UIStackView(arrangedSubviews: dots)
        dotRow.spacing = 3
        dotRow.alignment = .center
        for dot in dots {
            dot.backgroundColor = UIKitStyle.muted
            dot.layer.cornerRadius = 2
            dot.alpha = 0.45
            NSLayoutConstraint.activate([dot.widthAnchor.constraint(equalToConstant: 4), dot.heightAnchor.constraint(equalToConstant: 4)])
        }
        dotRow.isAccessibilityElement = false
        label.lineBreakMode = .byTruncatingTail
        let row = UIStackView(arrangedSubviews: [dotRow, label])
        row.spacing = 7
        row.alignment = .center
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        visible = heightAnchor.constraint(equalToConstant: 20)
        let hidden = heightAnchor.constraint(equalToConstant: 0)
        hidden.priority = .defaultHigh
        NSLayoutConstraint.activate([
            hidden,
            row.leadingAnchor.constraint(equalTo: leadingAnchor),
            row.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            row.centerYAnchor.constraint(equalTo: topAnchor, constant: 10),
        ])
        accessibilityIdentifier = "typing-indicator"
        isHidden = true
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    var text: String { label.text ?? "" }

    func show(_ text: String) {
        guard text != label.text || isHidden != text.isEmpty else { return }
        label.text = text
        isHidden = text.isEmpty
        visible.isActive = !text.isEmpty
        isAccessibilityElement = !text.isEmpty
        accessibilityLabel = text
        text.isEmpty ? stop() : animate()
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window != nil, !isHidden { animate() }
    }

    private func animate() {
        guard !UIKitStyle.reduceMotion, dots.first?.layer.animation(forKey: "bounce") == nil else { return }
        let now = CACurrentMediaTime()
        for (index, dot) in dots.enumerated() {
            let lift = CAKeyframeAnimation(keyPath: "transform.translation.y")
            lift.values = [0, -3, 0, 0]
            let fade = CAKeyframeAnimation(keyPath: "opacity")
            fade.values = [0.45, 1, 0.45, 0.45]
            let group = CAAnimationGroup()
            for animation in [lift, fade] { animation.keyTimes = [0, 0.25, 0.5, 1] }
            group.animations = [lift, fade]
            group.duration = 1.2
            group.repeatCount = .infinity
            group.beginTime = now + Double(index) * 0.15
            group.fillMode = .backwards
            dot.layer.add(group, forKey: "bounce")
        }
    }

    private func stop() { dots.forEach { $0.layer.removeAnimation(forKey: "bounce") } }
}
