import AVKit
import SwiftUI
import WebKit

// MARK: Link preview cards (web Embeds.tsx)

/// A message's link preview cards, drawn from their blocks. Cards present
/// when the row is first built show in place; a card that arrives while the
/// row is on screen (`message.embeds`) gets the short entrance.
struct MessageEmbedsView: View {
    let embeds: [ChatEmbed]
    var onRemove: (() -> Void)?
    /// Read once, when this row's view is first built.
    @State private var initial: Set<String>

    init(embeds: [ChatEmbed], animatesAll: Bool = false, onRemove: (() -> Void)? = nil) {
        self.embeds = embeds
        self.onRemove = onRemove
        _initial = State(initialValue: animatesAll ? [] : Set(embeds.map(\.url)))
    }

    var body: some View {
        if !embeds.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(embeds, id: \.url) { embed in
                    EmbedCardView(embed: embed, reservesRemove: onRemove != nil)
                        .modifier(Entrance.embed(!initial.contains(embed.url)))
                }
            }
            // Tracks never grow past the message column to fit 400pt media.
            .frame(maxWidth: 432, alignment: .leading)
            .overlay(alignment: .topTrailing) {
                if let onRemove { EmbedRemoveButton(action: onRemove) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .modifier(SwipeGuardedLinks())
        }
    }
}

/// The channel composer's card for the draft's first link, with × to send
/// without previews. Shown only for signed-in accounts (`composerPreviewLink`).
struct ComposerEmbedPreview: View {
    let chat: ChatModel

    var body: some View {
        if let embed = chat.composerLinkPreview {
            MessageEmbedsView(embeds: [embed], animatesAll: true) { chat.removeComposerLinkPreview() }
                .accessibilityIdentifier("composer-link-preview")
        }
    }
}

private struct EmbedRemoveButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "xmark").font(.system(size: 10, weight: .bold))
                .frame(width: 24, height: 24)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(CaperTheme.muted)
        .background(CaperTheme.raised, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .padding(4)
        .help("Remove link preview")
        .accessibilityLabel("Remove link preview")
        .accessibilityIdentifier("remove-link-preview")
    }
}

private struct EmbedCardView: View {
    let embed: ChatEmbed
    /// Room for the composer's remove button over the card's corner.
    var reservesRemove = false

    var body: some View {
        let heading = embed.heading
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 6) {
                EmbedSiteLine(embed: embed)
                ForEach(Array(embed.blocks.enumerated()), id: \.offset) { item in
                    EmbedBlockView(block: item.element, heading: heading, cardURL: embed.url)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if let thumbnail = embed.thumbnail {
                EmbedThumbnail(image: thumbnail, cardURL: embed.url)
            }
        }
        .padding(.top, 10)
        .padding(.leading, 12)
        .padding(.trailing, reservesRemove ? 36 : 12)
        .padding(.bottom, 12)
        .background(CaperTheme.surface, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text(verbatim: "\(embed.site.name): \(heading)"))
    }
}

/// Opens an embed's http(s) link in the system browser, like message links;
/// anything else draws the label without an action.
private struct EmbedLink<Label: View>: View {
    let href: String?
    @ViewBuilder let label: () -> Label
    @Environment(\.openURL) private var openURL

    var body: some View {
        if let url = LinkEmbeds.openableURL(href) {
            Button {
                // A drawer swipe that started on the card lifts on it too.
                guard !SwipeTapGuard.swiping else { return }
                openURL(url)
            } label: { label().contentShape(Rectangle()) }
            .buttonStyle(.plain)
            .accessibilityAddTraits(.isLink)
        } else {
            label()
        }
    }
}

private struct EmbedSiteLine: View {
    let embed: ChatEmbed

    var body: some View {
        EmbedLink(href: embed.url) {
            HStack(spacing: 6) {
                Text(embed.site.name).lineLimit(1)
                if embed.site.name != embed.site.domain {
                    Text(embed.site.domain).lineLimit(1).truncationMode(.tail).opacity(0.75)
                }
            }
            .font(CaperTheme.font(11))
            .foregroundStyle(CaperTheme.muted)
        }
    }
}

private struct EmbedBlockView: View {
    let block: EmbedBlock
    let heading: String
    let cardURL: String

    var body: some View {
        switch block {
        case .author(let author): EmbedAuthorRow(author: author)
        case .title(let title): EmbedTitleLine(title: title)
        case .text(let text): EmbedBodyText(text: text)
        case .images(let images):
            // A thumbnail sits beside the text instead (EmbedThumbnail).
            if !images.thumbnail { EmbedImagesView(images: images) }
        case .video(let video): EmbedVideoView(video: video, title: heading)
        case .player(let player): EmbedPlayerView(player: player, title: heading, cardURL: cardURL)
        case .footer(let footer): EmbedFooterLine(footer: footer)
        }
    }
}

private struct EmbedAuthorRow: View {
    let author: EmbedAuthor

    var body: some View {
        HStack(spacing: 8) {
            if let icon = author.icon {
                EmbedRemoteImage(url: icon, maxPixelSize: 80)
                    .frame(width: 20, height: 20)
                    .background(CaperTheme.raised)
                    .clipShape(Circle())
                    .accessibilityHidden(true)
            }
            EmbedLink(href: author.url) {
                HStack(spacing: 6) {
                    Text(author.name).font(CaperTheme.font(13, weight: .bold)).foregroundStyle(CaperTheme.text).lineLimit(1)
                    if let handle = author.handle, !handle.isEmpty {
                        Text(handle).font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).lineLimit(1).truncationMode(.tail)
                    }
                }
            }
        }
    }
}

private struct EmbedTitleLine: View {
    let title: EmbedTitle

    var body: some View {
        EmbedLink(href: title.url) {
            Text(title.text)
                .font(CaperTheme.font(14, weight: .bold))
                .foregroundStyle(CaperTheme.text)
                .multilineTextAlignment(.leading)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// Plain text with line breaks kept; links found by the shared rules open
/// externally like message links. Clamped to eight lines.
private struct EmbedBodyText: View {
    let text: String

    var body: some View {
        let linked = MessageLinks.renderedText(text)
        let shown = linked ?? Text(text)
        shown
            .font(CaperTheme.font(13))
            .foregroundStyle(CaperTheme.text)
            .tint(CaperTheme.link)
            .lineLimit(8)
            .modifier(MessageLinkPills(enabled: linked != nil))
    }
}

private struct EmbedFooterLine: View {
    let footer: EmbedFooter

    var body: some View {
        if let line = LinkEmbeds.footerLine(footer) {
            Text(line).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
        }
    }
}

// MARK: Media

/// Reserves the media's final size before it loads (web `fit`).
private struct EmbedMediaFrame: ViewModifier {
    let width: Double?
    let height: Double?

    @ViewBuilder func body(content: Content) -> some View {
        let size = LinkEmbeds.fit(width: width, height: height)
        Color.clear
            .aspectRatio(CGFloat(size.aspectRatio), contentMode: .fit)
            .overlay { content }
            .background(CaperTheme.raised)
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .frame(maxWidth: CGFloat(size.width), alignment: .leading)
    }
}

/// An image, avatar or poster from Caper's embed media host, decoded and
/// cached like attachment images.
private struct EmbedRemoteImage: View {
    let url: String
    var maxPixelSize = 1200
    @State private var image: CGImage?
    @State private var failed = false

    private var key: String { "embed:\(maxPixelSize):\(url)" }

    var body: some View {
        ZStack {
            if let shown = image ?? AttachmentImageLoader.shared.cached(key) {
                Image(decorative: shown, scale: 1).resizable().scaledToFill()
            } else if failed {
                Image(systemName: "photo").font(.system(size: 16)).foregroundStyle(CaperTheme.muted)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .clipped()
        .task(id: key) {
            guard image == nil, let source = LinkEmbeds.openableURL(url) else {
                if image == nil { failed = true }
                return
            }
            do {
                let loaded = try await AttachmentImageLoader.shared.load(key: key, url: source, maxPixelSize: maxPixelSize)
                image = loaded
                failed = false
            } catch {
                if !Task.isCancelled { failed = true }
            }
        }
    }
}

private struct EmbedImagesView: View {
    let images: EmbedImages

    var body: some View {
        if images.items.count == 1, let image = images.items.first {
            EmbedImageCell(image: image)
                .modifier(EmbedMediaFrame(width: image.width, height: image.height))
        } else {
            EmbedGallery(items: images.items)
        }
    }
}

private struct EmbedImageCell: View {
    let image: EmbedImage

    var body: some View {
        EmbedLink(href: image.url) {
            EmbedRemoteImage(url: image.url)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityLabel(Text(verbatim: image.alt.flatMap { $0.isEmpty ? nil : $0 } ?? "Image"))
    }
}

/// Two to four images in two columns, kept to the single image's 300pt height
/// (three put the first beside the other two).
private struct EmbedGallery: View {
    let items: [EmbedImage]

    var body: some View {
        Group {
            if items.count == 2 {
                HStack(spacing: 4) { cell(0); cell(1) }.frame(height: 220)
            } else if items.count == 3 {
                HStack(spacing: 4) {
                    cell(0)
                    VStack(spacing: 4) { cell(1); cell(2) }
                }
                .frame(height: 300)
            } else {
                VStack(spacing: 4) {
                    HStack(spacing: 4) { cell(0); cell(1) }
                    HStack(spacing: 4) { cell(2); cell(3) }
                }
                .frame(height: 300)
            }
        }
        .frame(maxWidth: .infinity)
    }

    private func cell(_ index: Int) -> some View {
        EmbedImageCell(image: items[min(index, items.count - 1)])
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(CaperTheme.raised)
            .clipShape(RoundedRectangle(cornerRadius: 6))
    }
}

private struct EmbedThumbnail: View {
    let image: EmbedImage
    let cardURL: String

    var body: some View {
        EmbedLink(href: cardURL) {
            EmbedRemoteImage(url: image.url, maxPixelSize: 240)
        }
        .frame(width: 80, height: 80)
        .background(CaperTheme.raised)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .accessibilityLabel(Text(verbatim: image.alt.flatMap { $0.isEmpty ? nil : $0 } ?? "Image"))
    }
}

private struct EmbedVideoView: View {
    let video: EmbedVideo
    let title: String

    var body: some View {
        Group {
            if video.gif {
                EmbedAnimationView(video: video, title: title)
            } else {
                EmbedPlayableVideo(video: video, title: title)
            }
        }
        .modifier(EmbedMediaFrame(width: video.width, height: video.height))
    }
}

/// A direct video, played inline in the system player after a tap, like an
/// attachment without the media viewer.
private struct EmbedPlayableVideo: View {
    let video: EmbedVideo
    let title: String
    @State private var player: AVPlayer?

    var body: some View {
        ZStack {
            if let player {
                VideoPlayer(player: player)
            } else {
                Button {
                    guard !SwipeTapGuard.swiping else { return }
                    start()
                } label: {
                    ZStack {
                        if let poster = video.poster { EmbedRemoteImage(url: poster) }
                        Image(systemName: "play.fill").font(.system(size: 18, weight: .bold)).foregroundStyle(.white)
                            .frame(width: 46, height: 46).background(Circle().fill(Color.black.opacity(0.6)))
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text(verbatim: "Play video \(title)"))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onDisappear { player?.pause() }
    }

    @MainActor private func start() {
        guard player == nil, let url = LinkEmbeds.openableURL(video.url) else { return }
        let created = AVPlayer(url: url)
        player = created
        created.play()
    }
}

/// A GIF-style video: muted, looping, without controls. With Reduce Motion
/// it stays on its poster until tapped; a tap pauses or resumes it.
private struct EmbedAnimationView: View {
    let video: EmbedVideo
    let title: String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var playback: LoopingPlayback?
    @State private var playing = false
    @State private var userPaused = false

    var body: some View {
        ZStack {
            if let poster = video.poster { EmbedRemoteImage(url: poster) }
            if let playback { PlayerSurface(player: playback.player) }
            if !playing {
                Image(systemName: "play.fill").font(.system(size: 14, weight: .bold)).foregroundStyle(.white)
                    .frame(width: 36, height: 36).background(Circle().fill(Color.black.opacity(0.6)))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .contentShape(Rectangle())
        .onTapGesture {
            guard !SwipeTapGuard.swiping else { return }
            toggle()
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: "Animation \(title)"))
        .accessibilityValue(playing ? "Playing" : "Paused")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { toggle() }
        .task(id: reduceMotion) {
            if reduceMotion || userPaused { pause() } else { await play() }
        }
        .onDisappear {
            playback?.stop()
            playback = nil
            playing = false
        }
    }

    @MainActor private func toggle() {
        if playing {
            userPaused = true
            pause()
        } else {
            userPaused = false
            Task { @MainActor in await play() }
        }
    }

    @MainActor private func pause() {
        playback?.player.pause()
        playing = false
    }

    @MainActor private func play() async {
        if let playback {
            playback.player.play()
            playing = true
            return
        }
        guard let url = LinkEmbeds.openableURL(video.url), !Task.isCancelled else { return }
        let created = LoopingPlayback(url: url)
        playback = created
        created.player.play()
        playing = true
        let loaded = await created.loads()
        guard !loaded, !Task.isCancelled, playback === created else { return }
        // The poster stays; a later tap tries again.
        created.stop()
        playback = nil
        playing = false
    }
}

/// A provider's player: its poster with Play. Play loads Caper's player page
/// (`play`) in place, only from Caper's embed host; otherwise it opens the link.
private struct EmbedPlayerView: View {
    let player: EmbedPlayer
    let title: String
    let cardURL: String
    @Environment(\.openURL) private var openURL
    @State private var playing = false

    var body: some View {
        ZStack {
            if playing, let page = LinkEmbeds.playURL(player.play) {
                EmbedPlayerWebView(url: page)
            } else {
                Button { play() } label: {
                    ZStack {
                        if let poster = player.poster { EmbedRemoteImage(url: poster) }
                        EmbedPlayButton()
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text(verbatim: "Play \(title)"))
            }
        }
        .modifier(EmbedMediaFrame(width: player.width, height: player.height))
    }

    @MainActor private func play() {
        guard !SwipeTapGuard.swiping else { return }
        if LinkEmbeds.playURL(player.play) != nil {
            playing = true
        } else if let url = LinkEmbeds.openableURL(cardURL) {
            openURL(url)
        }
    }
}

/// The eggshell primary play control (web .chat-embed-play).
private struct EmbedPlayButton: View {
    var body: some View {
        Image(systemName: "play.fill")
            .font(.system(size: 20, weight: .bold))
            .foregroundStyle(CaperTheme.onAction)
            .offset(x: 2)
            .frame(width: 52, height: 52)
            .background(Circle().fill(CaperTheme.action))
            .shadow(color: .black.opacity(0.4), radius: 6, y: 2)
    }
}

// MARK: Player web view

/// Caper's player page in a web view with no bridge to app code (no script
/// message handlers), no persistent storage and inline autoplay after Play.
/// Only the page itself loads in the main frame: any other navigation there,
/// and every new window, opens in the system browser. The provider's player
/// runs in the page's frames.
@MainActor
final class EmbedPlayerNavigation: NSObject, WKNavigationDelegate, WKUIDelegate {
    var openURL: OpenURLAction?

    static func makeWebView(url: URL, coordinator: EmbedPlayerNavigation) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.mediaTypesRequiringUserActionForPlayback = []
        #if os(iOS)
        configuration.allowsInlineMediaPlayback = true
        #endif
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.navigationDelegate = coordinator
        view.uiDelegate = coordinator
        #if os(iOS)
        view.isOpaque = false
        view.backgroundColor = .black
        view.scrollView.isScrollEnabled = false
        #endif
        view.load(URLRequest(url: url))
        return view
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction) async -> WKNavigationActionPolicy {
        guard let frame = navigationAction.targetFrame else {
            // A new window (`target=_blank`).
            openOutside(navigationAction.request.url)
            return .cancel
        }
        if !frame.isMainFrame { return .allow }
        if let url = navigationAction.request.url, LinkEmbeds.isPlayPage(url) { return .allow }
        openOutside(navigationAction.request.url)
        return .cancel
    }

    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        // `window.open`: never a second web view.
        openOutside(navigationAction.request.url)
        return nil
    }

    private func openOutside(_ url: URL?) {
        guard let url, let scheme = url.scheme?.lowercased(), scheme == "https" || scheme == "http", let openURL else { return }
        openURL(url)
    }
}

#if os(iOS)
private struct EmbedPlayerWebView: UIViewRepresentable {
    let url: URL

    func makeCoordinator() -> EmbedPlayerNavigation { EmbedPlayerNavigation() }

    func makeUIView(context: Context) -> WKWebView {
        context.coordinator.openURL = context.environment.openURL
        return EmbedPlayerNavigation.makeWebView(url: url, coordinator: context.coordinator)
    }

    func updateUIView(_ view: WKWebView, context: Context) {
        context.coordinator.openURL = context.environment.openURL
    }
}
#else
private struct EmbedPlayerWebView: NSViewRepresentable {
    let url: URL

    func makeCoordinator() -> EmbedPlayerNavigation { EmbedPlayerNavigation() }

    func makeNSView(context: Context) -> WKWebView {
        context.coordinator.openURL = context.environment.openURL
        return EmbedPlayerNavigation.makeWebView(url: url, coordinator: context.coordinator)
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        context.coordinator.openURL = context.environment.openURL
    }
}
#endif
