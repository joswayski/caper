import CaperCore
import CoreText
import SwiftUI
import UIKit

/// The conversation composer, as SwiftUI's phone layout draws it: one box
/// holding Attach (+), the native text view, Emojis/GIFs/Stickers and the
/// microphone, with Send beside it and staged files and voice notices above.
/// The text view is the shipping composer's own TextKit 1 UITextView: Markdown
/// delimiters keep their source offsets but draw nothing, so bold, italic,
/// mention and link highlights, caret moves, deletion and copy match SwiftUI,
/// and dictation, marked text (Japanese, Chinese, Korean IMEs), autocorrect,
/// undo and VoiceOver's value, caret and selection stay native.
final class ComposerView: UIView, UITextViewDelegate, UIGestureRecognizerDelegate {
    let textView: ComposerTextView = {
        let storage = NSTextStorage()
        let layout = DraftLinkLayoutManager()
        storage.addLayoutManager(layout)
        let container = NSTextContainer(size: CGSize(width: 0, height: CGFloat.greatestFiniteMagnitude))
        container.widthTracksTextView = true
        layout.addTextContainer(container)
        let view = ComposerTextView(frame: .zero, textContainer: container)
        layout.delegate = view.composerGlyphs
        return view
    }()
    /// Attribute and selection fixes made here, not by the person typing.
    private var highlighting = false
    /// The style web carries between typed keys (shared/messages/composer.rs).
    private var typing = ComposerTyping()
    /// The conversation the shown draft belongs to: another one never continues the typing style.
    private var conversation: String?
    private let placeholder = UIKitStyle.label(size: 14, color: UIKitStyle.muted)
    private let field = UIView()
    private let fieldRow = UIStackView()
    private lazy var attach = IconButton(icon: "plus", label: "Attach files", size: 18, side: 42) { [weak self] in self?.attachTapped?() }
    private lazy var emoji = IconButton(icon: "smile", label: "Emojis, GIFs and stickers", size: 18, side: 42) { [weak self] in self?.emojiTapped?() }
    private lazy var send = IconButton(icon: "send", label: "Send message", size: 16, side: 42) { [weak self] in self?.submit() }
    /// SwiftUI leaves: staged files and voice notices; the microphone; its recording bar.
    private let accessories = HostedView()
    private let microphone = HostedView()
    /// `@` and `:` suggestions from the shared autocomplete, above the box.
    private let suggestions = ComposerSuggestionsList()
    /// Escape closes them until the draft or caret moves again.
    private var dismissed = false
    private let recordingBar = HostedView()
    /// The draft's link card inside the box, and the length counter under it.
    /// A sizing host, so the box grows when the card loads after typing pauses.
    private let linkPreview: UIHostingController<AnyView> = {
        // Zero height until a card shows: an EmptyView host still reports a size.
        let host = UIHostingController(rootView: AnyView(Color.clear.frame(height: 0)))
        host.sizingOptions = .intrinsicContentSize
        host.view.backgroundColor = .clear
        host.view.translatesAutoresizingMaskIntoConstraints = false
        host.view.setContentHuggingPriority(.required, for: .vertical)
        host.view.setContentCompressionResistancePriority(.required, for: .vertical)
        return host
    }()
    private var linkPreviewShown = false
    private let counter = UIKitStyle.label(size: 10, color: UIKitStyle.muted)
    private var fieldToBottom: NSLayoutConstraint!
    private var fieldToCounter: [NSLayoutConstraint] = []
    private var height: NSLayoutConstraint!
    /// The shared draft rendering sets Satoshi 14 itself, as SwiftUI does.
    private let font = UIKitStyle.font(14, scaled: false)
    /// Text edits; the owner writes them to the model's draft.
    var changed: ((String) -> Void)?
    var submitted: (() -> Void)?
    var attachTapped: (() -> Void)?
    var emojiTapped: (() -> Void)?
    /// Who `@` may suggest: the open conversation's context.
    weak var model: AppModel?

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
        textView.textColor = UIKitStyle.text
        textView.tintColor = UIKitStyle.terracottaBright
        textView.keyboardAppearance = .dark
        textView.textContainerInset = UIEdgeInsets(top: 12, left: 0, bottom: 12, right: 0)
        textView.textContainer.lineFragmentPadding = 5
        textView.showsHorizontalScrollIndicator = false
        textView.delegate = self
        // As on the shipping iPhone composer, Return adds a line; Send sends.
        textView.accessibilityHint = "Use the Send button when composing with the iPhone keyboard."
        textView.autocompleteActive = { [weak self] in self?.suggestions.match != nil }
        textView.handleKey = { [weak self] in self?.handle($0) }
        textView.formatSelection = { [weak self] in self?.format($0) }
        let touchDown = TouchDownObserver(target: nil, action: nil)
        touchDown.cancelsTouchesInView = false
        touchDown.delaysTouchesEnded = false
        touchDown.touched = { [weak textView] in textView?.touched() }
        textView.addGestureRecognizer(touchDown)
        // Holding (or hovering) an emoji shows its :name:.
        let hold = UILongPressGestureRecognizer(target: textView, action: #selector(ComposerTextView.inspectEmojiHold(_:)))
        hold.minimumPressDuration = 0.5
        hold.allowableMovement = 10
        hold.cancelsTouchesInView = false
        hold.delegate = self
        textView.addGestureRecognizer(hold)
        textView.addGestureRecognizer(UIHoverGestureRecognizer(target: textView, action: #selector(ComposerTextView.inspectEmojiHover(_:))))
        suggestions.accepted = { [weak self] in self?.accept($0) }
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
        field.addSubview(linkPreview.view)
        counter.translatesAutoresizingMaskIntoConstraints = false
        // Satoshi with tabular digits, as SwiftUI's .monospacedDigit().
        let base = UIKitStyle.font(10, scaled: false)
        counter.font = UIFont(descriptor: base.fontDescriptor.addingAttributes([.featureSettings: [[
            UIFontDescriptor.FeatureKey.type: kNumberSpacingType, UIFontDescriptor.FeatureKey.selector: kMonospacedNumbersSelector,
        ]]]), size: 10)
        counter.isHidden = true
        addSubview(counter)
        recordingBar.isHidden = true
        field.addSubview(recordingBar)
        // Outside the clipped box, so the held microphone's lock hint can rise above it.
        addSubview(microphone)

        send.layer.cornerRadius = 8
        send.layer.cornerCurve = .continuous
        send.layer.borderWidth = 1
        send.accessibilityIdentifier = "send-message"
        // Its own disabled image, so UIKit doesn't darken the glyph: disabled
        // reads as SwiftUI's muted icon on raised chrome.
        send.setImage(send.image(for: .normal), for: .disabled)
        addSubview(send)
        addSubview(accessories)
        addSubview(suggestions)

        height = textView.heightAnchor.constraint(equalToConstant: 42)
        NSLayoutConstraint.activate([
            accessories.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            accessories.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
            accessories.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18),
            suggestions.topAnchor.constraint(equalTo: accessories.bottomAnchor),
            suggestions.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
            suggestions.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18),
            field.topAnchor.constraint(equalTo: suggestions.bottomAnchor),
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
            send.leadingAnchor.constraint(equalTo: field.trailingAnchor, constant: 8),
            send.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18),
            send.bottomAnchor.constraint(equalTo: field.bottomAnchor),
            fieldRow.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: 4),
            fieldRow.trailingAnchor.constraint(equalTo: microphone.leadingAnchor),
            fieldRow.topAnchor.constraint(equalTo: field.topAnchor),
            linkPreview.view.topAnchor.constraint(equalTo: fieldRow.bottomAnchor),
            linkPreview.view.leadingAnchor.constraint(equalTo: field.leadingAnchor),
            linkPreview.view.trailingAnchor.constraint(equalTo: field.trailingAnchor),
            linkPreview.view.bottomAnchor.constraint(equalTo: field.bottomAnchor),
            microphone.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -2),
            microphone.bottomAnchor.constraint(equalTo: fieldRow.bottomAnchor),
            microphone.widthAnchor.constraint(equalToConstant: 36),
            microphone.heightAnchor.constraint(equalToConstant: 42),
            // The bar covers the field beside the microphone, which stays under the finger.
            recordingBar.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: 8),
            recordingBar.trailingAnchor.constraint(equalTo: microphone.leadingAnchor),
            recordingBar.bottomAnchor.constraint(equalTo: fieldRow.bottomAnchor),
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
        fieldToBottom = field.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8)
        fieldToBottom.isActive = true
        fieldToCounter = [
            counter.topAnchor.constraint(equalTo: field.bottomAnchor, constant: 4),
            counter.centerXAnchor.constraint(equalTo: centerXAnchor),
            counter.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
        ]
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(prompt: String, enabled: Bool, canSend: Bool, sending: Bool, canAttach: Bool, attachReason: String?, canPickExpressions: Bool) {
        placeholder.text = prompt
        textView.accessibilityLabel = prompt
        textView.isEditable = enabled
        if !enabled { suggestions.show(nil); textView.hideEmojiName() }
        send.isEnabled = enabled && canSend
        send.accessibilityValue = sending ? "Sending" : nil
        // Disabled steps back to raised neutral chrome (below), not IconButton's fade.
        send.alpha = 1
        // Eggshell when there is something to send; otherwise the raised
        // neutral chrome of SwiftUI's PrimaryActionFill.
        send.backgroundColor = send.isEnabled ? UIKitStyle.action : UIKitStyle.raised
        send.tintColor = send.isEnabled ? UIKitStyle.blackout : UIKitStyle.muted
        send.layer.borderColor = (send.isEnabled ? UIKitStyle.action : UIKitStyle.border).cgColor
        attach.isEnabled = enabled && canAttach
        attach.accessibilityValue = attachReason
        emoji.isEnabled = enabled && canPickExpressions
    }

    /// The channel draft's link card (the thread composer has none).
    /// Set once per conversation: the hosted view follows the draft itself.
    func showLinkPreview<Content: View>(_ content: Content?) {
        guard (content != nil) != linkPreviewShown else { return }
        linkPreviewShown = content != nil
        linkPreview.rootView = content.map { AnyView($0) } ?? AnyView(Color.clear.frame(height: 0))
    }

    /// SwiftUI's "3,214 / 4,000" from 3,000 characters, warming near the limit.
    func showCount(_ count: Int) {
        let shown = count >= 3000
        guard shown || !counter.isHidden else { return }
        counter.text = "\(count.formatted()) / 4,000"
        counter.textColor = UIKitComposerCounter.color(count)
        counter.accessibilityLabel = "\(count.formatted()) of 4,000 characters"
        guard counter.isHidden == shown else { return }
        counter.isHidden = !shown
        if shown { fieldToBottom.isActive = false; NSLayoutConstraint.activate(fieldToCounter) }
        else { NSLayoutConstraint.deactivate(fieldToCounter); fieldToBottom.isActive = true }
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
    func show(draft: String, conversation: String) {
        if conversation != self.conversation { self.conversation = conversation; typing.end() }
        guard textView.markedTextRange == nil, textView.text != draft else { return }
        typing.end()
        textView.hideEmojiName()
        textView.text = draft
        dismissed = false
        refresh()
        render()
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

    /// Typing that would break a Markdown pair (deleting one `*` of `**`, say)
    /// is repaired as SwiftUI's composer repairs it.
    func textView(_ textView: UITextView, shouldChangeTextIn range: NSRange, replacementText replacement: String) -> Bool {
        guard !highlighting, textView.markedTextRange == nil,
              textView.undoManager?.isUndoing != true, textView.undoManager?.isRedoing != true else { return true }
        let proposed = (textView.text as NSString).replacingCharacters(in: range, with: replacement)
        let result = typing.repairEdit(textView.text, text: proposed,
                                       selection: NSRange(location: range.location + replacement.utf16.count, length: 0), replacing: range)
        guard result.text != proposed,
              let all = textView.textRange(from: textView.beginningOfDocument, to: textView.endOfDocument) else { return true }
        highlighting = true
        textView.replace(all, withText: result.text)
        textView.selectedRange = result.selection
        highlighting = false
        edited()
        return false
    }

    func textViewDidChange(_ textView: UITextView) {
        guard !highlighting else { return }
        self.textView.hideEmojiName()
        edited()
    }

    func textViewDidChangeSelection(_ textView: UITextView) {
        guard !highlighting else { return }
        typing.observe(textView.text, selection: textView.selectedRange)
        dismissed = false
        render()
    }

    func textViewDidBeginEditing(_ textView: UITextView) { render() }
    func textViewDidEndEditing(_ textView: UITextView) {
        typing.end()
        self.textView.hideEmojiName()
        suggestions.show(nil)
    }

    func scrollViewDidScroll(_ scrollView: UIScrollView) { textView.hideEmojiName() }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                           shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }

    /// Bold and Italic lead the edit menu, as in SwiftUI.
    func textView(_ textView: UITextView, editMenuForTextIn range: NSRange, suggestedActions: [UIMenuElement]) -> UIMenu? {
        guard textView.isEditable else { return nil }
        return UIMenu(children: [UIMenu(title: "", options: .displayInline, children: [
            UIAction(title: "B", discoverabilityTitle: "Bold") { [weak self] _ in self?.format("**") },
            UIAction(title: "I", discoverabilityTitle: "Italic") { [weak self] _ in self?.format("_") },
        ])] + suggestedActions)
    }

    /// Wraps or unwraps the selection in `marker` (Command-B, Command-I, the menu).
    private func format(_ marker: String) {
        guard textView.isEditable, textView.markedTextRange == nil,
              let result = MessageFormatting.toggle(textView.text, selection: textView.selectedRange, marker: marker),
              result.text.unicodeScalars.count <= 4000, result.text != textView.text,
              let all = textView.textRange(from: textView.beginningOfDocument, to: textView.endOfDocument) else { return }
        highlighting = true
        textView.replace(all, withText: result.text)
        textView.selectedRange = result.selection
        highlighting = false
        edited()
    }

    /// After any change to the draft: size, report, re-render, re-suggest.
    private func edited() {
        refresh()
        changed?(textView.text)
        dismissed = false
        render()
    }

    /// Keeps the caret off hidden delimiters, then draws the draft's
    /// formatting, mentions and links and updates the suggestions.
    private func render() {
        guard !highlighting else { return }
        if textView.markedTextRange == nil {
            highlighting = true
            textView.selectedRange = MessageFormatting.normalizedSelection(textView.text, selection: textView.selectedRange)
            if let model { highlightDraft(textView.textStorage, source: model.composerMentions) }
            for key in [NSAttributedString.Key.composerDelimiter, .obliqueness, .backgroundColor, .draftLink, .kern] {
                textView.typingAttributes.removeValue(forKey: key)
            }
            textView.typingAttributes[.font] = font
            textView.typingAttributes[.foregroundColor] = UIKitStyle.text
            highlighting = false
        }
        suggest()
    }
    private var mentions: MentionSource?

    /// Members can finish loading, or the conversation change, under the same
    /// draft: re-render its mentions and suggestions when the context moves.
    func refreshContext() {
        guard let model else { return }
        let next = model.composerMentions
        guard next != mentions else { return }
        mentions = next
        render()
    }

    /// The shared match for the caret: none while an input method composes,
    /// with a selection, or after Escape.
    func suggest() {
        guard !highlighting, let model, !dismissed, textView.isFirstResponder, textView.isEditable else { return suggestions.show(nil) }
        suggestions.show(NativePresentation.autocomplete(text: textView.text, selection: textView.selectedRange,
                                                         markedText: textView.markedTextRange != nil, model: model))
    }

    /// Up and Down move the selected row; Return and Tab accept it; Escape closes.
    private func handle(_ key: String) {
        guard suggestions.match != nil, textView.markedTextRange == nil else { return }
        switch key {
        case UIKeyCommand.inputUpArrow, UIKeyCommand.inputDownArrow: suggestions.move(down: key == UIKeyCommand.inputDownArrow)
        case UIKeyCommand.inputEscape: dismissed = true; suggestions.show(nil)
        default: accept(suggestions.selected)
        }
    }

    /// One undoable replacement of the draft, with the caret after the
    /// inserted name or emoji.
    private func accept(_ index: Int) {
        guard textView.isEditable, let match = suggestions.match,
              let result = NativePresentation.inserting(choice: index, in: textView.text, match: match),
              let all = textView.textRange(from: textView.beginningOfDocument, to: textView.endOfDocument) else { return }
        highlighting = true
        textView.replace(all, withText: result.text)
        textView.selectedRange = result.selection
        highlighting = false
        textView.becomeFirstResponder()
        edited()
    }

    private var focusing: UUID?
    /// SwiftUI's ReplyComposerFocus: Reply asks for the keyboard; keep trying
    /// until the composer can take it (a closing sheet, a sliding pane), then
    /// consume the request. Call from an observed render.
    func claimFocus(_ chat: ChatModel, inThread: Bool) {
        guard let request = chat.composerFocusRequest(inThread: inThread), focusing != request else { return }
        focusing = request
        func attempt(_ left: Int) {
            guard chat.composerFocusRequest(inThread: inThread) == request else { focusing = nil; return }
            if window != nil, textView.isEditable, presentedByNothing, textView.becomeFirstResponder() {
                chat.consumeComposerFocus(request)
                focusing = nil
            } else if left > 0 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { attempt(left - 1) }
            } else { focusing = nil }
        }
        attempt(60)
    }

    /// No sheet covers the composer's screen.
    private var presentedByNothing: Bool {
        var responder: UIResponder? = self
        while let next = responder?.next {
            if let controller = next as? UIViewController { return controller.presentedViewController == nil || controller.presentedViewController?.isBeingDismissed == true }
            responder = next
        }
        return true
    }

    private func submit() {
        // Committing marked text first keeps an IME's in-progress word in the message.
        if textView.markedTextRange != nil { textView.unmarkText() }
        submitted?()
    }

    private func refresh() {
        // As SwiftUI's composer: VoiceOver reads the draft as it will be sent.
        textView.accessibilityValue = textView.text
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

/// The composer's lip, as SwiftUI's ComposerLip: an 8pt shade over the bottom
/// of a timeline, black at 14% along the bottom edge fading out upward. It
/// takes no layout space and no touches.
final class ComposerLipView: UIView {
    override class var layerClass: AnyClass { CAGradientLayer.self }

    override init(frame: CGRect) {
        super.init(frame: frame)
        let gradient = layer as! CAGradientLayer
        gradient.colors = [UIColor.black.withAlphaComponent(0).cgColor, UIColor.black.withAlphaComponent(0.14).cgColor]
        isUserInteractionEnabled = false
        isAccessibilityElement = false
        translatesAutoresizingMaskIntoConstraints = false
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// Lays a lip over the bottom of `timeline`, inside `container`.
    static func attach(to timeline: UIView, in container: UIView) {
        let lip = ComposerLipView()
        container.addSubview(lip)
        NSLayoutConstraint.activate([
            lip.leadingAnchor.constraint(equalTo: timeline.leadingAnchor),
            lip.trailingAnchor.constraint(equalTo: timeline.trailingAnchor),
            lip.bottomAnchor.constraint(equalTo: timeline.bottomAnchor),
            lip.heightAnchor.constraint(equalToConstant: 8),
        ])
    }
}

/// "Maya is typing…" above the composer, after three bouncing dots like web's
/// keyframes: up and back over the first 60% of a 1.2 s cycle, each dot 0.15 s
/// behind the last. Core Animation runs it off the main thread; Reduce Motion
/// keeps the dots still. Reserves its height when nobody is typing.
final class TypingIndicatorView: UIView {
    private let label = UIKitStyle.label(size: 11, color: UIKitStyle.muted)
    private let dots = (0..<3).map { _ in UIView() }

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
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 16),
            row.leadingAnchor.constraint(equalTo: leadingAnchor),
            row.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            row.centerYAnchor.constraint(equalTo: topAnchor, constant: 8),
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

/// The composer's suggestions, as SwiftUI's ComposerSuggestionsView draws
/// them: flat 44-point rows in a bordered surface, at most four tall before it
/// scrolls; mentions span the composer and emoji stay 260 points wide. The
/// selected row (for hardware keys) takes the terracotta tint.
final class ComposerSuggestionsList: UIView {
    private(set) var match: ComposerAutocompleteMatch?
    private(set) var selected = 0
    var accepted: ((Int) -> Void)?
    private let box = UIView()
    private let scroll = UIScrollView()
    private let rows = UIStackView()
    private var height: NSLayoutConstraint!
    private var boxHeight: NSLayoutConstraint!
    private var narrow: NSLayoutConstraint!

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        box.translatesAutoresizingMaskIntoConstraints = false
        box.backgroundColor = UIKitStyle.surface
        box.layer.cornerRadius = 6
        box.layer.cornerCurve = .continuous
        box.layer.borderWidth = 1
        box.layer.borderColor = UIKitStyle.border.cgColor
        box.clipsToBounds = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.showsHorizontalScrollIndicator = false
        rows.axis = .vertical
        rows.translatesAutoresizingMaskIntoConstraints = false
        addSubview(box)
        box.addSubview(scroll)
        scroll.addSubview(rows)
        height = heightAnchor.constraint(equalToConstant: 0)
        boxHeight = box.heightAnchor.constraint(equalToConstant: 0)
        narrow = box.widthAnchor.constraint(equalToConstant: 260)
        let wide = box.trailingAnchor.constraint(equalTo: trailingAnchor)
        wide.priority = .defaultHigh
        NSLayoutConstraint.activate([
            height, boxHeight, wide,
            box.topAnchor.constraint(equalTo: topAnchor),
            box.leadingAnchor.constraint(equalTo: leadingAnchor),
            box.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            scroll.topAnchor.constraint(equalTo: box.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: box.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: box.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: box.trailingAnchor),
            rows.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor),
            rows.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor),
            rows.leadingAnchor.constraint(equalTo: scroll.frameLayoutGuide.leadingAnchor),
            rows.trailingAnchor.constraint(equalTo: scroll.frameLayoutGuide.trailingAnchor),
        ])
        isHidden = true
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// Rebuilds only when the match changes, which also resets the selection.
    func show(_ next: ComposerAutocompleteMatch?) {
        guard next != match else { return }
        match = next
        selected = 0
        rows.arrangedSubviews.forEach { $0.removeFromSuperview() }
        isHidden = next == nil
        guard let next else {
            height.constant = 0
            boxHeight.constant = 0
            return
        }
        switch next {
        case let .emoji(emoji):
            for (index, entry) in emoji.choices.enumerated() {
                rows.addArrangedSubview(SuggestionRow(emoji: entry) { [weak self] in self?.accepted?(index) })
            }
        case let .mention(mention):
            for (index, candidate) in mention.choices.enumerated() {
                rows.addArrangedSubview(SuggestionRow(candidate: candidate) { [weak self] in self?.accepted?(index) })
            }
        }
        narrow.isActive = !next.isMention
        accessibilityIdentifier = next.isMention ? "mention-suggestions" : "emoji-suggestions"
        let visible = min(CGFloat(next.count) * 44, 176)
        boxHeight.constant = visible
        // A 6-point gap above the composer box.
        height.constant = visible + 6
        scroll.contentOffset = .zero
        highlight()
    }

    func move(down: Bool) {
        guard let count = match?.count, count > 0 else { return }
        selected = (selected + (down ? 1 : count - 1)) % count
        highlight()
        if let row = rows.arrangedSubviews[safe: selected] {
            scroll.scrollRectToVisible(row.frame.insetBy(dx: 0, dy: -44), animated: false)
        }
    }

    private func highlight() {
        for (index, row) in rows.arrangedSubviews.enumerated() {
            row.backgroundColor = index == selected ? UIKitStyle.terracotta.withAlphaComponent(0.32) : .clear
        }
    }
}

private extension Array {
    subscript(safe index: Int) -> Element? { indices.contains(index) ? self[index] : nil }
}

/// One suggestion: a person's avatar over their display name and muted
/// `@username`, a special's `@` mark over its description, or an emoji's
/// artwork beside its `:shortcode:`. One VoiceOver button per row.
private final class SuggestionRow: UIControl {
    private let action: () -> Void

    convenience init(candidate: MentionCandidate, action: @escaping () -> Void) {
        let member = candidate.kind == .member
        let leading: UIView
        if member {
            let avatar = AvatarView(side: 28)
            avatar.show(name: candidate.displayName, avatarID: candidate.avatarId, picture: candidate.picture)
            leading = avatar
        } else {
            let mark = UIImageView(image: UIImage(systemName: "at", withConfiguration: UIImage.SymbolConfiguration(pointSize: 13, weight: .semibold)))
            mark.tintColor = UIKitStyle.muted
            mark.contentMode = .center
            leading = mark
        }
        let title = UIKitStyle.label(member ? candidate.displayName : "@\(candidate.username)", size: 13, weight: .medium)
        let detail = UIKitStyle.label(member ? "@\(candidate.username)" : candidate.displayName, size: 11, color: UIKitStyle.muted)
        let lines = UIStackView(arrangedSubviews: [title, detail])
        lines.axis = .vertical
        lines.spacing = 1
        self.init(leading: leading, side: 28, trailing: lines, action: action)
        accessibilityLabel = candidate.accessibilityLabel
        accessibilityIdentifier = "mention-suggestion-\(candidate.username.lowercased())"
    }

    convenience init(emoji entry: EmojiCatalogEntry, action: @escaping () -> Void) {
        let art = UIImageView()
        art.contentMode = .scaleAspectFit
        if let image = NativePresentation.emojiImage(for: entry) { art.image = UIImage(cgImage: image) }
        let name = UIKitStyle.label(":\(entry.name.replacingOccurrences(of: " ", with: "_")):", size: 13, weight: .medium)
        self.init(leading: art, side: 24, trailing: name, action: action)
        accessibilityLabel = "\(entry.name), emoji"
        accessibilityIdentifier = "emoji-suggestion-\(entry.id)"
    }

    private init(leading: UIView, side: CGFloat, trailing: UIView, action: @escaping () -> Void) {
        self.action = action
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let row = UIStackView(arrangedSubviews: [leading, trailing])
        row.spacing = side == 28 ? 10 : 12
        row.alignment = .center
        row.isUserInteractionEnabled = false
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 44),
            leading.widthAnchor.constraint(equalToConstant: side),
            leading.heightAnchor.constraint(equalToConstant: side),
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            row.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -10),
            row.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .button
        addAction(UIAction { [weak self] _ in self?.action() }, for: .touchUpInside)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var isHighlighted: Bool {
        didSet { alpha = isHighlighted ? 0.7 : 1 }
    }

    override func accessibilityActivate() -> Bool {
        action()
        return true
    }
}
