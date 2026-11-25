import Foundation
import Observation

/// The composer's backdrop over the end of its conversation (web's
/// `.chat-composer::before`). The timeline runs on under the typing row and
/// composer; the backdrop turns solid only `sink` points below the composer's
/// top edge, about its input's midline, and fades to clear over the `fade`
/// points above that: the conversation color alone, no blur material. The
/// conversation, and a hovered row, shows on beside the input's upper half,
/// so the input floats over it rather than on a slab.
public enum ComposerBackdrop {
    /// The eased band above the solid part.
    public static let fade: CGFloat = 32
    /// How far below the composer's top edge the backdrop turns solid.
    public static let sink: CGFloat = 22
    /// Web's stops: points down from the band's top, and the conversation color's opacity.
    public static let stops: [(offset: CGFloat, opacity: CGFloat)] = [(0, 0), (9, 0.3), (17, 0.62), (24, 0.86), (32, 1)]
}

/// The draft's heading name and excerpt; nil when there is no reply target.
/// A blocked author's message shows no name or excerpt.
@MainActor private func replyDraftText(_ chat: ChatModel, inThread: Bool) -> (name: String, excerpt: String, mentions: [MessageMention])? {
    guard let root = inThread ? chat.threadRootID : "channel", let reply = chat.replyTargets[root] else { return nil }
    let hidden = chat.blockedAuthorIDs.contains(reply.author.id) && reply.author.id != chat.viewerID
    let text = reply.fullText ?? reply.text
    return (hidden ? "blocked message" : reply.author.name, hidden ? "" : text.isEmpty
        ? VoiceMessage.excerptFallback(hasAttachments: reply.hasAttachments, hasVoice: reply.hasVoice, attachments: reply.attachments ?? []) : text,
        reply.mentions)
}

#if os(macOS)
import AppKit

/// Retains each accessory while the draft changes, including the voice retry
/// actions and staged files. Width is supplied by the native composer.
@MainActor final class AppKitComposerAccessories: NSStackView {
    var heightChanged: (() -> Void)?
    private let chat: ChatModel
    private let inThread: Bool
    private let onlyReply: Bool
    private let observation = AppKitObservation()
    private let notice = AppKitKit.label("", size: 11, color: AppKitKit.error)
    private let waiting = AppKitKit.label("Uploading… Your message sends when every file is ready.", size: 11, color: AppKitKit.muted)
    private let replyTitle = AppKitKit.label("", size: 12, wraps: false)
    private let replyText = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let reply = NSStackView()
    private let replyAccent = CALayer()
    private var cancel: AppKitKitButton!
    private let files: AppKitStagedFiles
    private let voice: AppKitVoiceControls
    init(chat: ChatModel, inThread: Bool, onlyReply: Bool = false, preview: @escaping (AttachmentDraft) -> Void) {
        self.chat = chat; self.inThread = inThread; self.onlyReply = onlyReply
        files = AppKitStagedFiles { NativeStagedFile.chat(chat, inThread: inThread, preview: preview) }
        voice = AppKitVoiceControls(session: NativeVoiceSession { .chat(chat, inThread: inThread) }, kind: .notice)
        super.init(frame: .zero); orientation = .vertical; alignment = .leading; spacing = 8
        // Up to two lines of the quoted message, ending in an ellipsis rather
        // than being cut off by the bar's edge.
        replyText.maximumNumberOfLines = 2; replyText.lineBreakMode = .byWordWrapping
        replyText.cell?.truncatesLastVisibleLine = true
        let words = AppKitKit.stack([replyTitle, replyText], spacing: 2)
        cancel = AppKitKitButton("", kind: .plain) {
            if let root = inThread ? chat.threadRootID : "channel" { chat.replyTargets[root] = nil }
        }
        cancel.image = NSImage(systemSymbolName: "xmark", accessibilityDescription: nil)?
            .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 12, weight: .semibold))
        cancel.imagePosition = .imageOnly; cancel.contentTintColor = AppKitKit.muted; cancel.setAccessibilityLabel("Cancel reply")
        NSLayoutConstraint.activate([cancel.widthAnchor.constraint(equalToConstant: 28), cancel.heightAnchor.constraint(equalToConstant: 28)])
        // The × is centred against the text and sits as far from the bar's
        // trailing edge as the text does from its leading edge.
        reply.orientation = .horizontal; reply.alignment = .centerY; reply.spacing = 8; reply.distribution = .fill
        reply.edgeInsets = NSEdgeInsets(top: 8, left: 12, bottom: 8, right: 6)
        reply.wantsLayer = true; reply.layer?.backgroundColor = NSColor(CaperTheme.replyDraft).cgColor
        reply.layer?.borderColor = AppKitKit.border.cgColor; reply.layer?.borderWidth = 1; reply.layer?.cornerRadius = 8
        reply.layer?.masksToBounds = true
        // Web's border-left accent, following the card's rounded edge.
        replyAccent.backgroundColor = CaperThemeStore.accentColor.cgColor; reply.layer?.addSublayer(replyAccent)
        reply.addArrangedSubview(words); reply.addArrangedSubview(cancel)
        // The text takes the bar's slack so × sits at the trailing edge.
        words.setContentHuggingPriority(.defaultLow, for: .horizontal)
        cancel.setContentHuggingPriority(.required, for: .horizontal)
        if onlyReply { addArrangedSubview(reply); AppKitKit.fillWidth(reply, of: self) }
        else {
            for view in [notice, voice, waiting, files] as [NSView] { addArrangedSubview(view); AppKitKit.fillWidth(view, of: self) }
        }
        waiting.setAccessibilityIdentifier(inThread ? "thread-upload-wait" : "composer-upload-wait")
        files.heightChanged = { [weak self] in self?.heightChanged?() }
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) }
        else { observation.stop() }
    }
    /// A wrapping label measures at its preferred width, so it gets the bar's
    /// text width before the surface asks for the fitting height.
    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        let width = max(1, newSize.width - reply.edgeInsets.left - reply.edgeInsets.right - reply.spacing - 28)
        if onlyReply, replyText.preferredMaxLayoutWidth != width { replyText.preferredMaxLayoutWidth = width; invalidateIntrinsicContentSize() }
    }
    override func layout() {
        super.layout()
        CATransaction.begin(); CATransaction.setDisableActions(true)
        replyAccent.frame = CGRect(x: 0, y: 0, width: 2, height: reply.bounds.height)
        CATransaction.commit()
    }
    private func render() {
        if onlyReply {
            let text = replyDraftText(chat, inThread: inThread)
            let heading = NSMutableAttributedString(string: "Replying to ", attributes: [.font: AppKitKit.font(12), .foregroundColor: AppKitKit.muted])
            heading.append(NSAttributedString(string: text?.name ?? "", attributes: [.font: AppKitKit.font(12, weight: .bold), .foregroundColor: AppKitKit.text]))
            replyTitle.attributedStringValue = heading
            replyText.attributedStringValue = ReplyMentions.excerpt(text?.excerpt ?? "", mentions: text?.mentions ?? [], font: AppKitKit.font(12), color: AppKitKit.muted)
            replyText.lineBreakMode = .byWordWrapping; replyText.cell?.truncatesLastVisibleLine = true
            replyAccent.backgroundColor = CaperThemeStore.accentColor.cgColor; cancel.contentTintColor = AppKitKit.muted
            replyText.isHidden = replyText.stringValue.isEmpty; reply.isHidden = text == nil
        } else {
            notice.stringValue = (inThread ? chat.threadAttachmentNotice : chat.attachmentNotice) ?? ""
            notice.isHidden = notice.stringValue.isEmpty; waiting.isHidden = !chat.uploadWaiting(inThread: inThread)
            files.isHidden = chat.attachmentFiles(inThread: inThread).isEmpty
            voice.render(); files.render()
        }
        invalidateIntrinsicContentSize(); heightChanged?()
    }
}

@MainActor final class AppKitComposerPreview: NSStackView {
    var heightChanged: (() -> Void)?
    private let chat: ChatModel
    private let inThread: Bool
    private let observation = AppKitObservation()
    private let card = NSStackView()
    private let site = AppKitKit.label("", size: 11, color: AppKitKit.muted, wraps: false)
    private let title = AppKitKit.label("", size: 13, weight: .medium, wraps: false)
    private let detail = AppKitKit.label("", size: 12, color: AppKitKit.muted, wraps: false)
    private let image = NSImageView()
    private var link: String?
    private var imageURL: String?
    private var load: Task<Void, Never>?
    private var imageLoad: Task<Void, Never>?
    init(chat: ChatModel, inThread: Bool) {
        self.chat = chat; self.inThread = inThread; super.init(frame: .zero)
        orientation = .vertical; alignment = .leading
        let text = AppKitKit.stack([site, title, detail], spacing: 2)
        image.imageScaling = .scaleProportionallyUpOrDown; image.setAccessibilityElement(false)
        image.widthAnchor.constraint(equalToConstant: 56).isActive = true
        image.heightAnchor.constraint(equalToConstant: 56).isActive = true
        let remove = AppKitKitButton("", kind: .plain) { chat.removeComposerLinkPreview(inThread: inThread) }
        remove.image = NSImage(systemSymbolName: "xmark", accessibilityDescription: nil); remove.imagePosition = .imageOnly
        remove.setAccessibilityLabel("Remove link preview")
        card.orientation = .horizontal; card.alignment = .top; card.spacing = 10
        card.edgeInsets = NSEdgeInsets(top: 8, left: 8, bottom: 8, right: 8)
        [image, text, remove].forEach(card.addArrangedSubview)
        card.wantsLayer = true; card.layer?.cornerRadius = 8; card.layer?.borderWidth = 1
        card.layer?.borderColor = AppKitKit.border.cgColor; card.layer?.backgroundColor = AppKitKit.raised.cgColor
        addArrangedSubview(card); AppKitKit.fillWidth(card, of: self)
        setAccessibilityIdentifier(inThread ? "thread-link-preview" : "composer-link-preview"); render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { load?.cancel(); imageLoad?.cancel() }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) }
        else { observation.stop(); load?.cancel(); link = nil }
    }
    private func render() {
        let next = chat.draftPreviewLink(inThread: inThread)
        if next != link { link = next; load?.cancel(); load = Task { [chat, inThread] in await chat.loadComposerLinkPreview(inThread: inThread) } }
        let embed = chat.draftLinkPreview(inThread: inThread), wasHidden = card.isHidden
        card.isHidden = embed == nil
        site.stringValue = embed.map { $0.site.name == $0.site.domain ? $0.site.name : "\($0.site.name)  \($0.site.domain)" } ?? ""
        title.stringValue = embed?.heading ?? ""; detail.stringValue = embed?.firstText ?? ""; detail.isHidden = detail.stringValue.isEmpty
        image.isHidden = embed?.compactImage == nil
        if imageURL != embed?.compactImage {
            imageURL = embed?.compactImage; imageLoad?.cancel(); image.image = nil
            if let url = imageURL.flatMap(LinkEmbeds.openableURL) {
                imageLoad = Task { [weak self] in
                    let value = try? await AttachmentImageLoader.shared.load(key: "embed:168:\(url.absoluteString)", url: url, maxPixelSize: 168)
                    guard !Task.isCancelled else { return }
                    self?.image.image = value.map { NSImage(cgImage: $0, size: NSSize(width: 56, height: 56)) }
                }
            }
        }
        if wasHidden, !card.isHidden, !CaperAccessibility.reduceMotion {
            let entrance = CABasicAnimation(keyPath: "opacity"); entrance.fromValue = 0; entrance.toValue = 1
            entrance.duration = CaperMotion.base
            entrance.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
            card.layer?.add(entrance, forKey: "entrance")
        }
        invalidateIntrinsicContentSize(); heightChanged?()
    }
}
#elseif os(iOS)
import UIKit

@MainActor public final class UIKitNativeComposerAccessories: UIStackView {
    private let chat: ChatModel
    private let inThread: Bool
    private let notice = UIKitMessageDialog.label("", error: true)
    private let waiting = UIKitMessageDialog.label("Uploading… Your message sends when every file is ready.")
    private let replyTitle = UIKitMessageDialog.label("", bold: true)
    private let replyText = UIKitMessageDialog.label("")
    private let reply = UIStackView()
    private let files: UIKitStagedFiles
    private let voice: UIKitVoiceControls
    private var active = false, generation = 0
    public init(chat: ChatModel, inThread: Bool = false, present: @escaping UIKitHosting.Present) {
        self.chat = chat; self.inThread = inThread
        files = UIKitStagedFiles {
            NativeStagedFile.chat(chat, inThread: inThread) { file in
                let holder = NativeComposerControllerBox()
                let close: () -> Void = { holder.value?.dismiss(animated: false) }
                let controller: UIViewController
                if let request = NativeStagedFile.editRequest(file, chat: chat, inThread: inThread) { controller = UIKitAttachmentEditor(request: request, close: close) }
                else { controller = UIKitFilePreview(url: file.localURL, close: close) }
                holder.value = controller; present(controller)
            }
        }
        voice = UIKitVoiceControls(session: NativeVoiceSession { .chat(chat, inThread: inThread) }, kind: .notice)
        super.init(frame: .zero); axis = .vertical; spacing = 8
        replyTitle.numberOfLines = 1; replyText.numberOfLines = 2
        let words = UIStackView(arrangedSubviews: [replyTitle, replyText]); words.axis = .vertical; words.spacing = 3
        let cancel = UIKitMessageDialog.button("×") { if let root = inThread ? chat.threadRootID : "channel" { chat.replyTargets[root] = nil } }
        cancel.accessibilityLabel = "Cancel reply"
        cancel.widthAnchor.constraint(equalToConstant: 44).isActive = true
        reply.axis = .horizontal; reply.alignment = .center; reply.spacing = 7
        reply.layoutMargins = UIEdgeInsets(top: 8, left: 10, bottom: 8, right: 0); reply.isLayoutMarginsRelativeArrangement = true
        reply.backgroundColor = UIColor(CaperTheme.replyDraft); reply.layer.cornerRadius = 8
        reply.layer.borderColor = UIColor(CaperTheme.border).cgColor; reply.layer.borderWidth = 1
        reply.addArrangedSubview(words); reply.addArrangedSubview(cancel)
        [reply, notice, voice, waiting, files].forEach(addArrangedSubview)
        waiting.accessibilityIdentifier = inThread ? "thread-upload-wait" : "composer-upload-wait"
        files.heightChanged = { [weak self] in self?.invalidateIntrinsicContentSize() }; render()
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    public override func didMoveToWindow() { super.didMoveToWindow(); active = window != nil; generation += 1; if active { observe(generation) } }
    private func observe(_ token: Int) { guard active, token == generation else { return }; withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } } }
    private func render() {
        let text = replyDraftText(chat, inThread: inThread)
        let heading = NSMutableAttributedString(string: "Replying to ", attributes: [.font: replyText.font!, .foregroundColor: UIColor(CaperTheme.muted)])
        heading.append(NSAttributedString(string: text?.name ?? "", attributes: [.font: replyTitle.font!, .foregroundColor: UIColor(CaperTheme.text)]))
        replyTitle.attributedText = heading
        replyText.attributedText = ReplyMentions.excerpt(text?.excerpt ?? "", mentions: text?.mentions ?? [], font: replyText.font, color: UIColor(CaperTheme.muted))
        replyText.lineBreakMode = .byTruncatingTail
        replyText.isHidden = text?.excerpt.isEmpty != false; reply.isHidden = text == nil
        notice.text = inThread ? chat.threadAttachmentNotice : chat.attachmentNotice; notice.isHidden = notice.text == nil
        waiting.isHidden = !chat.uploadWaiting(inThread: inThread); files.isHidden = chat.attachmentFiles(inThread: inThread).isEmpty
        voice.render(); files.render(); invalidateIntrinsicContentSize()
    }
}

private final class NativeComposerControllerBox { weak var value: UIViewController? }

@MainActor public final class UIKitNativeComposerPreview: UIStackView {
    private let chat: ChatModel
    private let inThread: Bool
    private let card = UIStackView()
    private let site = UIKitMessageDialog.label("", size: 11)
    private let title = UIKitMessageDialog.label("", size: 13)
    private let detail = UIKitMessageDialog.label("", size: 12)
    private let image = UIImageView()
    private var link: String?
    private var imageURL: String?
    private var load: Task<Void, Never>?
    private var imageLoad: Task<Void, Never>?
    private var active = false, generation = 0
    public init(chat: ChatModel, inThread: Bool = false) {
        self.chat = chat; self.inThread = inThread; super.init(frame: .zero)
        axis = .vertical
        [site, title, detail].forEach { $0.numberOfLines = 1 }
        let words = UIStackView(arrangedSubviews: [site, title, detail]); words.axis = .vertical; words.spacing = 2
        image.contentMode = .scaleAspectFill; image.clipsToBounds = true; image.layer.cornerRadius = 6
        image.widthAnchor.constraint(equalToConstant: 56).isActive = true; image.heightAnchor.constraint(equalToConstant: 56).isActive = true
        let remove = UIKitMessageDialog.button("×") { chat.removeComposerLinkPreview(inThread: inThread) }
        remove.accessibilityLabel = "Remove link preview"; remove.widthAnchor.constraint(equalToConstant: 44).isActive = true
        [image, words, remove].forEach(card.addArrangedSubview)
        card.alignment = .top; card.spacing = 10; card.isLayoutMarginsRelativeArrangement = true
        card.layoutMargins = UIEdgeInsets(top: 8, left: 8, bottom: 8, right: 4)
        card.backgroundColor = UIColor(CaperTheme.raised); card.layer.borderColor = UIColor(CaperTheme.border).cgColor
        card.layer.borderWidth = 1; card.layer.cornerRadius = 8
        addArrangedSubview(card); accessibilityIdentifier = inThread ? "thread-link-preview" : "composer-link-preview"; render()
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { load?.cancel(); imageLoad?.cancel() }
    public override func didMoveToWindow() {
        super.didMoveToWindow(); active = window != nil; generation += 1
        if active { observe(generation) } else { load?.cancel(); link = nil }
    }
    private func observe(_ token: Int) { guard active, token == generation else { return }; withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } } }
    private func render() {
        let next = chat.draftPreviewLink(inThread: inThread)
        if next != link { link = next; load?.cancel(); load = Task { [chat, inThread] in await chat.loadComposerLinkPreview(inThread: inThread) } }
        let embed = chat.draftLinkPreview(inThread: inThread), wasHidden = card.isHidden
        card.isHidden = embed == nil
        site.text = embed.map { $0.site.name == $0.site.domain ? $0.site.name : "\($0.site.name)  \($0.site.domain)" }
        title.text = embed?.heading; detail.text = embed?.firstText; detail.isHidden = detail.text == nil
        image.isHidden = embed?.compactImage == nil
        if imageURL != embed?.compactImage {
            imageURL = embed?.compactImage; imageLoad?.cancel(); image.image = nil
            if let url = imageURL.flatMap(LinkEmbeds.openableURL) {
                imageLoad = Task { [weak self] in
                    let value = try? await AttachmentImageLoader.shared.load(key: "embed:168:\(url.absoluteString)", url: url, maxPixelSize: 168)
                    guard !Task.isCancelled else { return }; self?.image.image = value.map { UIImage(cgImage: $0) }
                }
            }
        }
        if wasHidden, !card.isHidden, !CaperAccessibility.reduceMotion {
            card.alpha = 0
            let animator = UIViewPropertyAnimator(duration: CaperMotion.base,
                controlPoint1: CGPoint(x: 0.16, y: 1), controlPoint2: CGPoint(x: 0.3, y: 1)) { self.card.alpha = 1 }
            animator.startAnimation()
        }
        invalidateIntrinsicContentSize()
    }
}
#endif
