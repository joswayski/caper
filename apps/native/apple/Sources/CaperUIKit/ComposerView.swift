import CaperCore
import UIKit

/// The conversation composer: a native, TextKit 2 UITextView, so dictation,
/// marked text (Japanese, Chinese, Korean IMEs), autocorrect and VoiceOver's
/// value, caret and selection all come from UIKit itself.
final class ComposerView: UIView, UITextViewDelegate {
    let textView = ComposerTextView(usingTextLayoutManager: true)
    private let placeholder = UIKitStyle.label(size: 16, color: UIKitStyle.muted)
    private let field = UIView()
    private lazy var send = IconButton(icon: "send", label: "Send message", size: 20, side: 48) { [weak self] in self?.submit() }
    private var height: NSLayoutConstraint!
    private let font = UIKitStyle.font(16)
    /// Text edits; the owner writes them to the model's draft.
    var changed: ((String) -> Void)?
    var submitted: (() -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        field.translatesAutoresizingMaskIntoConstraints = false
        field.backgroundColor = UIKitStyle.composer.withAlphaComponent(0.55)
        field.layer.cornerRadius = 8
        field.layer.cornerCurve = .continuous
        field.layer.borderWidth = 1
        field.layer.borderColor = UIKitStyle.border.cgColor
        addSubview(field)

        textView.translatesAutoresizingMaskIntoConstraints = false
        textView.backgroundColor = .clear
        textView.font = font
        textView.adjustsFontForContentSizeCategory = true
        textView.textColor = UIKitStyle.text
        textView.tintColor = UIKitStyle.terracottaBright
        textView.keyboardAppearance = .dark
        textView.textContainerInset = UIEdgeInsets(top: 13, left: 0, bottom: 13, right: 0)
        textView.textContainer.lineFragmentPadding = 0
        textView.showsHorizontalScrollIndicator = false
        textView.delegate = self
        textView.returnPressed = { [weak self] in self?.submit() }
        textView.accessibilityIdentifier = "message-composer"
        field.addSubview(textView)
        // The text view's own label carries the prompt.
        placeholder.accessibilityElementsHidden = true
        field.addSubview(placeholder)

        send.layer.cornerRadius = 8
        send.layer.borderWidth = 1
        send.layer.borderColor = UIKitStyle.border.cgColor
        send.accessibilityIdentifier = "send-message"
        addSubview(send)

        height = textView.heightAnchor.constraint(equalToConstant: 48)
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
            field.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            field.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
            send.leadingAnchor.constraint(equalTo: field.trailingAnchor, constant: 10),
            send.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18),
            send.bottomAnchor.constraint(equalTo: field.bottomAnchor),
            textView.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: 16),
            textView.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -12),
            textView.topAnchor.constraint(equalTo: field.topAnchor),
            textView.bottomAnchor.constraint(equalTo: field.bottomAnchor),
            height,
            placeholder.leadingAnchor.constraint(equalTo: textView.leadingAnchor),
            placeholder.centerYAnchor.constraint(equalTo: field.topAnchor, constant: 24),
            placeholder.trailingAnchor.constraint(lessThanOrEqualTo: textView.trailingAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(channelName: String, enabled: Bool, canSend: Bool) {
        let prompt = "Message #\(channelName)"
        placeholder.text = prompt
        textView.accessibilityLabel = prompt
        textView.isEditable = enabled
        send.isEnabled = enabled && canSend
    }

    /// The model's draft, written back only when it differs (a send clears it)
    /// and never while an input method is composing.
    func show(draft: String) {
        guard textView.markedTextRange == nil, textView.text != draft else { return }
        textView.text = draft
        refresh()
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
        let fitting = textView.sizeThatFits(CGSize(width: textView.bounds.width > 0 ? textView.bounds.width : 280, height: .greatestFiniteMagnitude)).height
        // One line to six, then it scrolls.
        let maximum = 26 + lineHeight * 6
        let next = min(max(48, ceil(fitting)), maximum)
        textView.isScrollEnabled = fitting > maximum
        if height.constant != next { height.constant = next }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        refresh()
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
