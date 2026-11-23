import AVKit
import SwiftUI
import WebKit

#if os(macOS)
private typealias EmbedNativeView = NSView
#else
private typealias EmbedNativeView = UIView
#endif

private enum NativeEmbedSource {
    case image(EmbedImage, link: String)
    case video(EmbedVideo)
    case player(EmbedPlayer, fallback: String)
    var size: (width: Double, aspectRatio: Double) {
        switch self {
        case .image(let image, _): return LinkEmbeds.fit(width: image.width, height: image.height)
        case .video(let video): return LinkEmbeds.fit(width: video.width, height: video.height)
        case .player(let player, _): return LinkEmbeds.fit(width: player.width, height: player.height)
        }
    }
    var poster: String? {
        switch self { case .image(let image, _): image.url; case .video(let video): video.poster; case .player(let player, _): player.poster }
    }
}

/// Provider pages still use WebKit, with the existing host allowlist and
/// ephemeral data store. The card, poster and player controls are native.
@MainActor private final class NativeEmbedMedia: EmbedNativeView {
    private let source: NativeEmbedSource
    private let title: String
    private let picture = CALayer(), animation = AVPlayerLayer()
    private var pictureTask: Task<Void, Never>?, playTask: Task<Void, Never>?
    private var loop: LoopingPlayback?
    private var player: AVPlayer?
    private var web: WKWebView?
    private let navigation = EmbedPlayerNavigation()
    private var userPaused = false, active = false, playing = false
    #if os(macOS)
    private let button = AppKitMediaHitTarget()
    private let playerView = AVPlayerView()
    /// A looping GIF clip holds still while the window is minimized or covered.
    private lazy var visibility = AppKitWindowVisibility { [weak self] in self?.visibilityChanged() }
    #else
    private let button = UIButton(type: .custom)
    private let playerController = AVPlayerViewController()
    #endif
    init(_ source: NativeEmbedSource, title: String) {
        self.source = source; self.title = title
        super.init(frame: .zero)
        #if os(macOS)
        wantsLayer = true; layer?.backgroundColor = AppKitKit.raised.cgColor; layer?.cornerRadius = 6; layer?.masksToBounds = true
        layer?.addSublayer(picture); layer?.addSublayer(animation)
        button.handler = { [weak self] in self?.activate() }; button.setAccessibilityLabel(title)
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(motionChanged), name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)
        #else
        backgroundColor = UIColor(CaperTheme.raised); layer.cornerRadius = 6; clipsToBounds = true
        layer.addSublayer(picture); layer.addSublayer(animation)
        button.addTarget(self, action: #selector(activate), for: .touchUpInside); button.accessibilityLabel = title
        NotificationCenter.default.addObserver(self, selector: #selector(motionChanged), name: UIAccessibility.reduceMotionStatusDidChangeNotification, object: nil)
        #endif
        picture.contentsGravity = .resizeAspectFill; picture.masksToBounds = true; animation.videoGravity = .resizeAspectFill
        addSubview(button); navigation.openURL = OpenURLAction { url in nativeOpenEmbed(url); return .handled }
        updateButton()
        // Height follows the width the layout gives (capped at the media's own
        // size), so a cell measuring itself at any width gets the right height.
        // Below required, so explicit sizes (thumbnails, gallery rows) win.
        let ratio = 1 / source.size.aspectRatio
        let fitted = heightAnchor.constraint(equalTo: widthAnchor, multiplier: ratio); fitted.priority = .defaultHigh
        let byWidth = heightAnchor.constraint(lessThanOrEqualTo: widthAnchor, multiplier: ratio); byWidth.priority = .init(999)
        let cap = heightAnchor.constraint(lessThanOrEqualToConstant: source.size.width * ratio); cap.priority = .init(999)
        NSLayoutConstraint.activate([fitted, byWidth, cap])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit {
        pictureTask?.cancel(); playTask?.cancel(); loop?.stop(); player?.pause()
        #if os(macOS)
        NSWorkspace.shared.notificationCenter.removeObserver(self)
        #else
        NotificationCenter.default.removeObserver(self)
        #endif
    }
    #if os(macOS)
    override var isFlipped: Bool { true }
    override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); visibility.track(window); attachmentChanged(window != nil) }
    override func layout() { super.layout(); layoutMedia() }
    private func visibilityChanged() {
        guard active, playing, let loop else { return }
        if visibility.visible { loop.player.play() } else { loop.player.pause() }
    }
    #else
    override func didMoveToWindow() { super.didMoveToWindow(); attachmentChanged(window != nil) }
    override func layoutSubviews() { super.layoutSubviews(); layoutMedia() }
    #endif
    private var shows: Bool {
        #if os(macOS)
        visibility.visible
        #else
        true
        #endif
    }
    private func attachmentChanged(_ attached: Bool) {
        active = attached
        if !attached {
            pictureTask?.cancel(); playTask?.cancel(); loop?.stop(); loop = nil; animation.player = nil; player?.pause()
            web?.stopLoading(); web?.removeFromSuperview(); web = nil
            #if os(iOS)
            playerController.willMove(toParent: nil); playerController.view.removeFromSuperview(); playerController.removeFromParent()
            #endif
            playing = false; updateButton(); return
        }
        if picture.contents == nil, let url = LinkEmbeds.openableURL(source.poster) {
            pictureTask = Task { [weak self] in
                let image = try? await AttachmentImageLoader.shared.load(key: "embed:1200:\(url.absoluteString)", url: url, maxPixelSize: 1200)
                guard !Task.isCancelled else { return }; self?.picture.contents = image
            }
        }
        if case .video(let video) = source, video.gif && !CaperAccessibility.reduceMotion && !userPaused { startLoop(video) }
        if player != nil { attachPlayer() }
    }
    @objc private func motionChanged() {
        guard active, case .video(let video) = source, video.gif else { return }
        if CaperAccessibility.reduceMotion {
            playTask?.cancel(); playTask = nil; loop?.stop(); loop = nil
            animation.player = nil; playing = false; updateButton()
        }
        else if !userPaused { startLoop(video) }
    }
    @objc private func activate() {
        guard active, !SwipeTapGuard.swiping else { return }
        switch source {
        case .image(_, let link): if let url = LinkEmbeds.openableURL(link) { nativeOpenEmbed(url) }
        case .video(let video):
            if video.gif {
                if playing { userPaused = true; playing = false; loop?.player.pause(); updateButton() }
                else { userPaused = false; startLoop(video, automatic: false) }
            } else if let url = LinkEmbeds.openableURL(video.url) {
                if player == nil { player = AVPlayer(url: url) }; attachPlayer(); player?.play(); updateButton()
            }
        case .player(let provider, let fallback):
            if let url = LinkEmbeds.playURL(provider.play) {
                web = EmbedPlayerNavigation.makeWebView(url: url, coordinator: navigation)
                if let web { addSubview(web) }; layoutMedia(); updateButton()
            } else if let url = LinkEmbeds.openableURL(fallback) { nativeOpenEmbed(url) }
        }
    }
    private func startLoop(_ video: EmbedVideo, automatic: Bool = true) {
        guard !automatic || !CaperAccessibility.reduceMotion else { return }
        if let loop { if shows { loop.player.play() }; playing = true; updateButton(); return }
        guard let url = LinkEmbeds.openableURL(video.url) else { return }
        let loop = LoopingPlayback(url: url); self.loop = loop; animation.player = loop.player
        playTask = Task { [weak self] in
            let loaded = await loop.loads()
            guard !Task.isCancelled, let self, self.loop === loop, active else { return }
            if loaded, !automatic || !CaperAccessibility.reduceMotion, !userPaused { playing = true; if shows { loop.player.play() } }
            else { loop.stop(); self.loop = nil; animation.player = nil; playing = false }
            updateButton()
        }
    }
    private func attachPlayer() {
        #if os(macOS)
        playerView.player = player; if playerView.superview == nil { addSubview(playerView) }
        #else
        playerController.player = player
        if playerController.parent == nil {
            var responder: UIResponder? = next
            while let current = responder {
                if let parent = current as? UIViewController {
                    parent.addChild(playerController); addSubview(playerController.view); playerController.didMove(toParent: parent); break
                }
                responder = current.next
            }
        }
        #endif
        layoutMedia()
    }
    private func updateButton() {
        let label: String, icon: String?
        switch source {
        case .image: label = title; icon = nil
        case .video(let video): label = video.gif ? "Animation \(title)" : "Play video \(title)"; icon = playing ? nil : "play.fill"
        case .player: label = "Play \(title)"; icon = "play.fill"
        }
        button.isHidden = player != nil || web != nil
        #if os(macOS)
        button.setAccessibilityLabel(label); button.setAccessibilityValue(playing ? "Playing" : "Paused")
        button.symbol = icon
        #else
        button.accessibilityLabel = label; button.accessibilityValue = playing ? "Playing" : "Paused"
        button.setImage(icon.flatMap(UIImage.init(systemName:)), for: .normal); button.tintColor = UIColor(CaperTheme.action)
        #endif
    }
    private func layoutMedia() {
        CATransaction.begin(); CATransaction.setDisableActions(true)
        let frame = CGRect(x: 0, y: 0, width: min(bounds.width, source.size.width), height: bounds.height)
        picture.frame = frame; animation.frame = frame; button.frame = frame; web?.frame = frame
        #if os(macOS)
        playerView.frame = frame
        #else
        if playerController.isViewLoaded { playerController.view.frame = frame }
        #endif
        CATransaction.commit()
    }
}

@MainActor private func nativeOpenEmbed(_ url: URL) {
    #if os(macOS)
    NSWorkspace.shared.open(url)
    #else
    UIApplication.shared.open(url)
    #endif
}

#if os(macOS)
/// A transparent, accessible hit target over media drawn by layers. A plain
/// untitled NSButton otherwise vanishes from AppKit's accessibility tree.
@MainActor final class AppKitMediaHitTarget: NSButton {
    var handler: (() -> Void)?
    var symbol: String? { didSet { needsDisplay = true } }
    init() { super.init(frame: .zero); isBordered = false; title = ""; target = self; action = #selector(press) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func press() { handler?() }
    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .button }
    override func accessibilityHitTest(_ point: NSPoint) -> Any? { self }
    override func draw(_ dirtyRect: NSRect) {
        if let symbol, let image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil) {
            image.draw(in: NSRect(x: (bounds.width - 26) / 2, y: (bounds.height - 26) / 2, width: 26, height: 26))
        }
        if window?.firstResponder === self { AppKitKit.terracottaBright.setStroke(); NSBezierPath(roundedRect: bounds.insetBy(dx: 1, dy: 1), xRadius: 6, yRadius: 6).stroke() }
    }
    override func resetCursorRects() { if isEnabled { addCursorRect(bounds, cursor: .pointingHand) } }
}

@MainActor final class AppKitMessageEmbeds: NSStackView {
    private var snapshots: [ChatEmbed] = []
    private var cards: [AppKitEmbedCard] = []
    init() { super.init(frame: .zero); orientation = .vertical; alignment = .leading; spacing = 6 }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ embeds: [ChatEmbed]) {
        guard snapshots != embeds else { return }
        let next = embeds.enumerated().map { index, embed in
            snapshots.indices.contains(index) && snapshots[index] == embed ? cards[index] : AppKitEmbedCard(embed)
        }
        for card in cards where !next.contains(where: { $0 === card }) { card.removeFromSuperview() }
        for (index, card) in next.enumerated() where card.superview == nil {
            insertArrangedSubview(card, at: index)
            card.translatesAutoresizingMaskIntoConstraints = false
            card.widthAnchor.constraint(lessThanOrEqualToConstant: 432).isActive = true
            let width = card.widthAnchor.constraint(equalTo: widthAnchor); width.priority = .defaultHigh; width.isActive = true
        }
        snapshots = embeds; cards = next
    }
}

@MainActor private final class AppKitEmbedCard: NSStackView {
    init(_ embed: ChatEmbed) {
        super.init(frame: .zero); orientation = .horizontal; alignment = .top; spacing = 12
        wantsLayer = true; layer?.backgroundColor = AppKitKit.surface.cgColor; layer?.borderColor = AppKitKit.border.cgColor; layer?.borderWidth = 1; layer?.cornerRadius = 8
        edgeInsets = NSEdgeInsets(top: 10, left: 12, bottom: 12, right: 12)
        let body = AppKitKit.stack([], spacing: 6); addArrangedSubview(body)
        let site = link(embed.site.name == embed.site.domain ? embed.site.name : "\(embed.site.name)  \(embed.site.domain)", url: embed.url, size: 11,
                        color: AppKitKit.muted)
        body.addArrangedSubview(site); AppKitKit.fillWidth(site, of: body)
        for block in embed.blocks {
            let row: NSView?
            switch block {
            case .author(let author):
                let name = link(author.name + (author.handle.map { "  \($0)" } ?? ""), url: author.url, size: 13)
                if let icon = author.icon {
                    let image = NativeEmbedMedia(.image(EmbedImage(url: icon, width: 20, height: 20, alt: nil), link: author.url ?? icon), title: author.name)
                    image.widthAnchor.constraint(equalToConstant: 20).isActive = true; image.heightAnchor.constraint(equalToConstant: 20).isActive = true
                    let line = NSStackView(views: [image, name]); line.spacing = 8; row = line
                } else { row = name }
            case .title(let title): row = link(title.text, url: title.url, size: 14, bold: true)
            case .text(let value): let text = AppKitMessageBody(); text.maximumLines = 8; text.update(NativePresentation.messageText(value, size: 13)); row = text
            case .images(let images): row = images.thumbnail ? nil : gallery(images.items)
            case .video(let video): row = NativeEmbedMedia(.video(video), title: embed.heading)
            case .player(let player): row = NativeEmbedMedia(.player(player, fallback: embed.url), title: embed.heading)
            case .footer(let footer): row = LinkEmbeds.footerLine(footer).map { AppKitKit.label($0, size: 11, color: AppKitKit.muted) }
            }
            if let row { body.addArrangedSubview(row); AppKitKit.fillWidth(row, of: body) }
        }
        if let thumbnail = embed.thumbnail {
            let image = NativeEmbedMedia(.image(thumbnail, link: embed.url), title: thumbnail.alt ?? "Image")
            image.widthAnchor.constraint(equalToConstant: 80).isActive = true; image.heightAnchor.constraint(equalToConstant: 80).isActive = true; addArrangedSubview(image)
        }
        setAccessibilityElement(false); setAccessibilityLabel("\(embed.site.name): \(embed.heading)")
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    /// Web's card links keep their text's colour, with no underline.
    private func link(_ text: String, url: String?, size: CGFloat, bold: Bool = false, color: NSColor? = nil) -> NSView {
        let tint = color ?? AppKitKit.text
        let view = AppKitMessageBody(), value = NSMutableAttributedString(string: text, attributes: [.font: AppKitKit.font(size, weight: bold ? .bold : .regular), .foregroundColor: tint])
        if let url = LinkEmbeds.openableURL(url) { value.addAttribute(.link, value: url, range: NSRange(location: 0, length: value.length)) }
        view.linkTextAttributes = [.foregroundColor: tint, .cursor: NSCursor.pointingHand]
        view.update(value); return view
    }
    private func gallery(_ items: [EmbedImage]) -> NSView {
        let cells = items.map { NativeEmbedMedia(.image($0, link: $0.url), title: $0.alt ?? "Image") }
        if cells.count == 1 { return cells[0] }
        if cells.count == 3 {
            let column = NSStackView(views: [cells[1], cells[2]]); column.orientation = .vertical; column.spacing = 4; column.distribution = .fillEqually
            let row = NSStackView(views: [cells[0], column]); row.spacing = 4; row.distribution = .fillEqually
            row.heightAnchor.constraint(equalToConstant: 300).isActive = true
            return row
        }
        let rows = AppKitKit.stack([], spacing: 4)
        for start in stride(from: 0, to: cells.count, by: 2) {
            let row = NSStackView(views: Array(cells[start..<min(start + 2, cells.count)])); row.spacing = 4; row.distribution = .fillEqually
            row.heightAnchor.constraint(equalToConstant: cells.count == 2 ? 220 : 148).isActive = true
            rows.addArrangedSubview(row); AppKitKit.fillWidth(row, of: rows)
        }
        return rows
    }
}
#else
@MainActor final class UIKitMessageEmbeds: UIStackView {
    private var snapshots: [ChatEmbed] = []
    private var cards: [UIKitEmbedCard] = []
    init() { super.init(frame: .zero); axis = .vertical; alignment = .leading; spacing = 6 }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ embeds: [ChatEmbed]) {
        guard snapshots != embeds else { return }
        let next = embeds.enumerated().map { index, embed in
            snapshots.indices.contains(index) && snapshots[index] == embed ? cards[index] : UIKitEmbedCard(embed)
        }
        for card in cards where !next.contains(where: { $0 === card }) { card.removeFromSuperview() }
        for (index, card) in next.enumerated() where card.superview == nil {
            insertArrangedSubview(card, at: index)
            card.widthAnchor.constraint(lessThanOrEqualToConstant: 432).isActive = true
            let width = card.widthAnchor.constraint(equalTo: widthAnchor); width.priority = .defaultHigh; width.isActive = true
        }
        snapshots = embeds; cards = next
    }
}

@MainActor private final class UIKitEmbedCard: UIStackView {
    init(_ embed: ChatEmbed) {
        super.init(frame: .zero); axis = .horizontal; alignment = .top; spacing = 12
        backgroundColor = UIColor(CaperTheme.surface); layer.borderColor = UIColor(CaperTheme.border).cgColor; layer.borderWidth = 1; layer.cornerRadius = 8
        isLayoutMarginsRelativeArrangement = true; directionalLayoutMargins = NSDirectionalEdgeInsets(top: 10, leading: 12, bottom: 12, trailing: 12)
        let body = UIStackView(); body.axis = .vertical; body.spacing = 6; addArrangedSubview(body)
        body.addArrangedSubview(link(embed.site.name == embed.site.domain ? embed.site.name : "\(embed.site.name)  \(embed.site.domain)", url: embed.url, size: 11,
                                     color: UIColor(CaperTheme.muted)))
        for block in embed.blocks {
            let row: UIView?
            switch block {
            case .author(let author):
                let name = link(author.name + (author.handle.map { "  \($0)" } ?? ""), url: author.url, size: 13)
                if let icon = author.icon {
                    let image = NativeEmbedMedia(.image(EmbedImage(url: icon, width: 20, height: 20, alt: nil), link: author.url ?? icon), title: author.name)
                    image.widthAnchor.constraint(equalToConstant: 20).isActive = true; image.heightAnchor.constraint(equalToConstant: 20).isActive = true
                    let line = UIStackView(arrangedSubviews: [image, name]); line.spacing = 8; row = line
                } else { row = name }
            case .title(let title): row = link(title.text, url: title.url, size: 14, bold: true)
            case .text(let value): let text = UIKitMessageBody(); text.update(NativePresentation.messageText(value, size: 13)); text.textContainer.maximumNumberOfLines = 8; row = text
            case .images(let images): row = images.thumbnail ? nil : gallery(images.items)
            case .video(let video): row = NativeEmbedMedia(.video(video), title: embed.heading)
            case .player(let player): row = NativeEmbedMedia(.player(player, fallback: embed.url), title: embed.heading)
            case .footer(let footer): row = LinkEmbeds.footerLine(footer).map { UIKitMessageDialog.label($0, size: 11) }
            }
            if let row { body.addArrangedSubview(row) }
        }
        if let thumbnail = embed.thumbnail {
            let image = NativeEmbedMedia(.image(thumbnail, link: embed.url), title: thumbnail.alt ?? "Image")
            image.widthAnchor.constraint(equalToConstant: 80).isActive = true; image.heightAnchor.constraint(equalToConstant: 80).isActive = true; addArrangedSubview(image)
        }
        accessibilityLabel = "\(embed.site.name): \(embed.heading)"; isAccessibilityElement = false
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    /// Web's card links keep their text's colour, not the green link tint.
    private func link(_ text: String, url: String?, size: CGFloat, bold: Bool = false, color: UIColor = UIColor(CaperTheme.text)) -> UIView {
        let view = UIKitMessageBody(), value = NSMutableAttributedString(string: text, attributes: [.font: UIFont(name: bold ? "Figtree-Bold" : "Figtree-Regular", size: size) ?? .systemFont(ofSize: size), .foregroundColor: color])
        if let url = LinkEmbeds.openableURL(url) { value.addAttribute(.link, value: url, range: NSRange(location: 0, length: value.length)) }
        view.linkTextAttributes = [.foregroundColor: color]
        view.update(value); return view
    }
    private func gallery(_ items: [EmbedImage]) -> UIView {
        let cells = items.map { NativeEmbedMedia(.image($0, link: $0.url), title: $0.alt ?? "Image") }
        if cells.count == 1 { return cells[0] }
        if cells.count == 3 {
            let column = UIStackView(arrangedSubviews: [cells[1], cells[2]]); column.axis = .vertical; column.spacing = 4; column.distribution = .fillEqually
            let row = UIStackView(arrangedSubviews: [cells[0], column]); row.spacing = 4; row.distribution = .fillEqually
            row.heightAnchor.constraint(equalToConstant: 300).isActive = true
            return row
        }
        let rows = UIStackView(); rows.axis = .vertical; rows.spacing = 4
        for start in stride(from: 0, to: cells.count, by: 2) {
            let row = UIStackView(arrangedSubviews: Array(cells[start..<min(start + 2, cells.count)])); row.spacing = 4; row.distribution = .fillEqually
            row.heightAnchor.constraint(equalToConstant: cells.count == 2 ? 220 : 148).isActive = true; rows.addArrangedSubview(row)
        }
        return rows
    }
}
#endif
