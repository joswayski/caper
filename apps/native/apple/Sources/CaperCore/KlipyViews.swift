import Observation
import SwiftUI
#if os(iOS)
import UIKit
import WebKit
#endif

// MARK: Animated KLIPY media

/// Plays a KLIPY animation (WebP, else GIF) through ImageIO, like inline GIF
/// attachments. Reduce Motion shows `still`, or else the first frame.
/// Loads straight from KLIPY's URLs, never through Caper's CDN signer.
struct KlipyAnimatedImage: View {
    let sources: [URL]
    let still: URL?
    let maxPixelSize: Int
    let animationByteLimit: Int
    @Binding var failed: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var original: AttachmentOriginal?
    @State private var started = Date()

    var body: some View {
        ZStack {
            if let original, let first = original.images.first {
                if reduceMotion || original.images.count == 1 {
                    Image(decorative: first, scale: 1).resizable().scaledToFit()
                } else {
                    TimelineView(.animation(minimumInterval: 0.02)) { timeline in
                        let index = MediaViewerPolicy.frameIndex(at: timeline.date.timeIntervalSince(started), durations: original.durations)
                        Image(decorative: original.images[index], scale: 1).resizable().scaledToFit()
                    }
                }
            } else if !failed {
                ProgressView().controlSize(.small)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task(id: "\(reduceMotion):\(sources.map(\.absoluteString).joined(separator: " "))") { await load() }
        .onDisappear { original = nil }
    }

    private func load() async {
        let loader = AttachmentImageLoader.shared
        if reduceMotion {
            // A still: KLIPY's, else the animation's first frame.
            for url in [still].compactMap({ $0 }) + sources {
                if let image = try? await loader.load(key: "klipy-still:\(url.absoluteString)", url: url, maxPixelSize: maxPixelSize) {
                    guard !Task.isCancelled else { return }
                    original = AttachmentOriginal(images: [image], durations: []); failed = false
                    return
                }
                if Task.isCancelled { return }
            }
        } else {
            for url in sources {
                if let frames = try? await loader.loadFrames(key: "klipy:\(url.absoluteString)", url: url, maxPixelSize: maxPixelSize,
                                                             animationByteLimit: animationByteLimit), !frames.images.isEmpty {
                    guard !Task.isCancelled else { return }
                    original = frames; started = Date(); failed = false
                    return
                }
                if Task.isCancelled { return }
            }
        }
        if !Task.isCancelled { failed = true }
    }
}

/// `content.gif` below a message's text: its aspect ratio, at most 300pt on
/// the longer edge (stickers 160pt, without a background).
struct MessageGifView: View {
    let gif: ChatGif
    @State private var failed = false
    @Environment(\.mediaViewer) private var mediaViewer
    @Environment(\.openURL) private var openURL

    var body: some View {
        Button {
            guard !SwipeTapGuard.swiping else { return }
            guard let item = gif.mediaViewerAttachment else { return }
            if let mediaViewer {
                mediaViewer(MediaViewerRequest(items: [item], start: 0, chat: nil, hideSpoiler: { _ in }))
            } else if let raw = item.url, let url = URL(string: raw) {
                openURL(url)
            }
        } label: { media.contentShape(Rectangle()) }
        .buttonStyle(.plain)
        .accessibilityLabel("Open \(gif.accessibilityLabel)")
    }

    @ViewBuilder private var media: some View {
        let width = CGFloat(gif.width), height = CGFloat(gif.height)
        let edge: CGFloat = gif.kind == .sticker ? 160 : 300
        let scale = min(1, edge / max(width, height))
        if failed {
            HStack(spacing: 6) {
                Image(systemName: "photo").font(.system(size: 12))
                Text("GIF unavailable").font(CaperTheme.font(12))
            }
            .foregroundStyle(CaperTheme.muted)
            .padding(.horizontal, 10).padding(.vertical, 7)
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("message-gif-unavailable")
        } else {
            Color.clear
                .aspectRatio(width / height, contentMode: .fit)
                .frame(maxWidth: width * scale, maxHeight: height * scale, alignment: .topLeading)
                .overlay {
                    KlipyAnimatedImage(sources: gif.animatedSources, still: gif.still.flatMap(URL.init(string:)),
                                       maxPixelSize: 600, animationByteLimit: 24 * 1024 * 1024, failed: $failed)
                }
                .background(gif.kind == .sticker ? Color.clear : CaperTheme.surface)
                .clipShape(RoundedRectangle(cornerRadius: gif.kind == .sticker ? 0 : 8))
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(gif.accessibilityLabel)
                .accessibilityAddTraits(.isImage)
                .accessibilityIdentifier("message-gif")
        }
    }
}

// MARK: Picker

@MainActor @Observable
final class KlipyPickerModel {
    var kind: ChatGif.Kind = .gif { didSet { if oldValue != kind { reload(debounce: false) } } }
    var query = "" { didSet { if oldValue != query { reload(debounce: true) } } }
    private(set) var items: [KlipyItem] = []
    private(set) var hasNext = false
    private(set) var loading = false
    /// The first page answered the current tab and search.
    private(set) var loaded = false
    private(set) var error: String?
    /// Changes when a new first page replaces the results, resetting the scroll.
    private(set) var resultsID = 0
    /// The search the shown results answer; empty for trending.
    private(set) var resultsQuery = ""
    /// The grid's width, for the ads' maximum width.
    var gridWidth: CGFloat = 0
    @ObservationIgnored private var page = 0
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var task: Task<Void, Never>?
    private let client: KlipyClient

    init(client: KlipyClient, kind: ChatGif.Kind = .gif) {
        self.client = client
        self.kind = kind
    }

    func start() {
        guard task == nil, !loaded else { return }
        reload(debounce: false)
    }

    /// Abandons any request in flight; `start` resumes from a fresh first page if none had loaded.
    func stop() {
        task?.cancel(); task = nil
        generation += 1
        loading = false
    }

    /// Searches wait 300 ms after typing stops; the shown results stay until the new ones arrive.
    func reload(debounce: Bool) {
        task?.cancel()
        generation += 1
        let generation = generation
        loading = true; error = nil
        task = Task { [weak self] in
            if debounce {
                try? await Task.sleep(for: .milliseconds(300))
                guard !Task.isCancelled else { return }
            }
            await self?.load(page: 1, generation: generation)
        }
    }

    /// The next page, while KLIPY has one.
    func loadMore() {
        guard loaded, hasNext, !loading, error == nil else { return }
        loading = true
        let generation = generation, next = page + 1
        task = Task { [weak self] in await self?.load(page: next, generation: generation) }
    }

    func retry() {
        if loaded && !items.isEmpty { error = nil; loadMore() } else { reload(debounce: false) }
    }

    private func load(page: Int, generation: Int) async {
        let query = self.query.trimmingCharacters(in: .whitespacesAndNewlines), kind = self.kind
        #if os(iOS)
        let ads: KlipyAdRequest? = await KlipyDevice.adRequest(gridWidth: gridWidth)
        #else
        // Desktop never asks for ads.
        let ads: KlipyAdRequest? = nil
        #endif
        guard generation == self.generation else { return }
        do {
            let result = try await client.page(kind: kind, query: query, page: page, ads: ads)
            guard generation == self.generation else { return }
            if page == 1 {
                items = result.items; resultsQuery = query; loaded = true; resultsID += 1
            } else {
                items += result.items
            }
            self.page = page; hasNext = result.hasNext; loading = false
        } catch {
            guard generation == self.generation else { return }
            if page == 1 { items = []; loaded = false; hasNext = false }
            self.error = "Couldn’t load from KLIPY."
            loading = false
        }
    }
}

/// GIFs | Stickers, a "Search KLIPY" field, and KLIPY's results in its own
/// order (ads included on iPhone). Choosing one hands it to `select` with
/// the search it came from.
struct KlipyPicker: View {
    let select: (ChatGif, String) -> Void
    var embeddedKind: ChatGif.Kind? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var model: KlipyPickerModel
    #if os(macOS)
    private let columns = 3
    #else
    private let columns = 2
    #endif

    init(configuration: KlipyConfiguration, embeddedKind: ChatGif.Kind? = nil, select: @escaping (ChatGif, String) -> Void) {
        self.select = select
        self.embeddedKind = embeddedKind
        _model = State(initialValue: KlipyPickerModel(client: KlipyClient(configuration: configuration), kind: embeddedKind ?? .gif))
    }

    var body: some View {
        #if os(macOS)
        VStack(spacing: 0) {
            HStack {
                Text("GIFs").font(CaperTheme.font(14, weight: .bold))
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
            }.padding(.horizontal, 12).padding(.top, 12)
            content
        }
        .frame(width: 400, height: 480)
        .accessibilityElement(children: .contain)
        .contentShape(.accessibility, Rectangle())
        .accessibilityIdentifier("gif-picker")
        #else
        if let embeddedKind {
            content
                .onAppear { model.kind = embeddedKind }
                .onChange(of: embeddedKind) { _, kind in model.kind = kind }
        } else {
            NavigationStack {
                content
                    .navigationTitle("GIFs")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { dismiss() }
                        }
                    }
            }
            .frame(minWidth: 320, minHeight: 420)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("gif-picker")
        }
        #endif
    }

    private var content: some View {
        VStack(spacing: 0) {
            if embeddedKind == nil {
                Picker("Type", selection: $model.kind) {
                    Text("GIFs").tag(ChatGif.Kind.gif)
                    Text("Stickers").tag(ChatGif.Kind.sticker)
                }
                .pickerStyle(.segmented).labelsHidden()
                .padding(.horizontal, 12).padding(.top, 12)
                .accessibilityIdentifier("gif-picker-kind")
            }
            // KLIPY requires this exact placeholder.
            TextField("Search KLIPY", text: $model.query)
                .textFieldStyle(.roundedBorder)
                .autocorrectionDisabled()
                #if os(iOS)
                .textInputAutocapitalization(.never)
                .submitLabel(.search)
                #endif
                .accessibilityIdentifier("gif-picker-search")
                .padding(12)
            results
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { model.gridWidth = max(0, $0 - 24) }
            Text("Powered by KLIPY").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                .frame(maxWidth: .infinity).padding(.vertical, 8)
                .accessibilityIdentifier("gif-picker-attribution")
        }
        .task { model.start() }
        .onDisappear { model.stop() }
    }

    @ViewBuilder private var results: some View {
        if model.items.isEmpty, let error = model.error {
            VStack(spacing: 10) {
                Text(error).font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).multilineTextAlignment(.center)
                Button("Try again") { model.retry() }.buttonStyle(CaperSecondaryButton())
                    .accessibilityIdentifier("gif-picker-retry")
            }.padding(24)
        } else if model.items.isEmpty && (model.loading || !model.loaded) {
            ProgressView().controlSize(.small)
        } else if model.items.isEmpty {
            Text("No results").font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                .accessibilityIdentifier("gif-picker-empty")
        } else {
            let rows = KlipyGridRow.rows(model.items, columns: columns)
            ScrollView {
                LazyVStack(spacing: 8) {
                    ForEach(rows) { row in
                        rowView(row).onAppear { if row.id == rows.last?.id { model.loadMore() } }
                    }
                    if let error = model.error {
                        HStack(spacing: 8) {
                            Text(error).foregroundStyle(CaperTheme.error)
                            Button("Retry") { model.retry() }
                        }.font(CaperTheme.font(11)).padding(.vertical, 6)
                    } else if model.loading {
                        ProgressView().controlSize(.small).padding(.vertical, 6)
                    }
                }
                .padding(.horizontal, 12).padding(.bottom, 8)
            }
            .id(model.resultsID)
            .accessibilityIdentifier("gif-picker-grid")
        }
    }

    @ViewBuilder private func rowView(_ row: KlipyGridRow) -> some View {
        switch row {
        case let .tiles(_, results):
            HStack(spacing: 8) {
                ForEach(0..<columns, id: \.self) { column in
                    if column < results.count {
                        KlipyTileButton(result: results[column]) { select(results[column].gif, model.resultsQuery) }
                    } else {
                        Color.clear.aspectRatio(1, contentMode: .fit).frame(maxWidth: .infinity).accessibilityHidden(true)
                    }
                }
            }
        case let .ad(_, ad):
            #if os(iOS)
            KlipyAdRow(ad: ad)
            #else
            // macOS never requests ads; ignore any that arrive.
            let _ = ad
            EmptyView()
            #endif
        }
    }
}

private struct KlipyTileButton: View {
    let result: KlipyResult
    let action: () -> Void
    @State private var failed = false

    var body: some View {
        Button(action: action) {
            Color.clear.aspectRatio(1, contentMode: .fit)
                .overlay {
                    if failed {
                        Image(systemName: "photo").font(.system(size: 16)).foregroundStyle(CaperTheme.muted)
                    } else {
                        KlipyAnimatedImage(sources: URL(string: result.tile.url).map { [$0] } ?? [],
                                           still: result.tile.still.flatMap(URL.init(string:)),
                                           maxPixelSize: 360, animationByteLimit: 4 * 1024 * 1024, failed: $failed)
                    }
                }
                .frame(maxWidth: .infinity)
                .background(result.gif.kind == .sticker ? Color.clear : CaperTheme.raised)
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .modifier(ControlHover())
        .accessibilityLabel(result.gif.accessibilityLabel)
        .accessibilityHint("Sends it now")
        .accessibilityIdentifier("gif-picker-item-\(result.slug)")
    }
}

/// The GIF control beside a composer's emoji button: a popover on macOS and a
/// sheet on iPhone. Choosing a GIF sends it at once; the draft stays.
struct ComposerGifButton: View {
    let configuration: KlipyConfiguration
    var enabled = true
    var controller: ComposerAutocompleteController? = nil
    var accessibilityID = "composer-gif"
    /// Whether the picker is showing, for composers that track open dialogs.
    var presenting: ((Bool) -> Void)? = nil
    let send: (ChatGif, String) -> Void
    @State private var showing = false

    var body: some View {
        Button { showing = true } label: {
            Text("GIF").font(CaperTheme.font(10, weight: .bold)).foregroundStyle(CaperTheme.muted)
                .padding(.horizontal, 3).frame(height: 16)
                .overlay(RoundedRectangle(cornerRadius: 4).stroke(CaperTheme.muted, lineWidth: 1.5))
                .frame(width: 36, height: 42).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .opacity(enabled ? 1 : 0.45)
        .help("Send a GIF")
        .accessibilityLabel("Send a GIF")
        .accessibilityIdentifier(accessibilityID)
        #if os(macOS)
        .popover(isPresented: $showing) { picker.onDisappear(perform: finish) }
        #else
        .sheet(isPresented: $showing, onDismiss: finish) {
            picker.presentationDetents([.medium, .large]).presentationBackground(CaperTheme.surface)
        }
        #endif
        .onChange(of: showing) { _, value in presenting?(value) }
    }

    private var picker: some View {
        KlipyPicker(configuration: configuration) { gif, query in
            showing = false
            send(gif, query)
        }
        .preferredColorScheme(.dark)
    }

    private func finish() { controller?.restoreFocus() }
}

/// Best effort, after a successful send: tells KLIPY which GIF was shared.
@MainActor
enum KlipyShare {
    static func send(_ gif: ChatGif, query: String, configuration: KlipyConfiguration) {
        let client = KlipyClient(configuration: configuration)
        Task {
            #if os(iOS)
            let agent = await KlipyDevice.userAgent()
            #else
            let agent: String? = nil
            #endif
            await client.share(kind: gif.kind, slug: gif.slug, query: query, userAgent: agent)
        }
    }
}

#if os(iOS)
// MARK: iPhone ads

/// The ad fields KLIPY recommends that need no permission prompt, and the
/// system WebView's user agent (KLIPY serves ads only to mobile agents).
@MainActor
enum KlipyDevice {
    private static var cachedUserAgent: String?
    private static var userAgentTask: Task<String?, Never>?

    /// WKWebView's `navigator.userAgent`, read once and kept for the process.
    static func userAgent() async -> String? {
        if let cachedUserAgent { return cachedUserAgent }
        let task = userAgentTask ?? Task { @MainActor in
            let configuration = WKWebViewConfiguration()
            configuration.websiteDataStore = .nonPersistent()
            let view = WKWebView(frame: .zero, configuration: configuration)
            let agent: String? = await withCheckedContinuation { continuation in
                view.evaluateJavaScript("navigator.userAgent") { value, _ in
                    continuation.resume(returning: value as? String)
                }
            }
            withExtendedLifetime(view) {}
            return agent
        }
        userAgentTask = task
        let agent = await task.value
        userAgentTask = nil
        if let agent, !agent.isEmpty { cachedUserAgent = agent }
        return cachedUserAgent
    }

    static func adRequest(gridWidth: CGFloat) async -> KlipyAdRequest {
        let screen = UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.screen }.first
        let size = screen?.bounds.size ?? .zero
        let width = gridWidth > 0 ? gridWidth : size.width - 24
        let language = Locale.preferredLanguages.first.flatMap { Locale(identifier: $0).language.languageCode?.identifier }
        return KlipyAdRequest(maxWidth: Int(width.rounded(.down)), osVersion: UIDevice.current.systemVersion, model: modelIdentifier,
                              deviceWidth: Int(size.width), deviceHeight: Int(size.height), pixelRatio: Double(screen?.scale ?? 1),
                              language: language, userAgent: await userAgent())
    }

    /// The hardware model, such as `iPhone16,2`.
    private static var modelIdentifier: String {
        if let simulated = ProcessInfo.processInfo.environment["SIMULATOR_MODEL_IDENTIFIER"] { return simulated }
        var system = utsname()
        uname(&system)
        return withUnsafeBytes(of: &system.machine) { raw in String(decoding: raw.prefix { $0 != 0 }, as: UTF8.self) }
    }
}

/// A full grid row labelled Sponsored, sized to the ad or else 100pt tall.
private struct KlipyAdRow: View {
    let ad: KlipyAd
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Sponsored").font(CaperTheme.font(10, weight: .bold)).foregroundStyle(CaperTheme.muted)
            KlipyAdWebView(html: ad.html)
                .frame(maxWidth: .infinity)
                .frame(height: CGFloat(ad.height ?? 100))
                .clipShape(RoundedRectangle(cornerRadius: 8))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("gif-picker-ad")
    }
}

/// The ad's HTML in an isolated web view: a non-persistent data store, no
/// script message handlers and no Caper origin (`about:blank`), so it can
/// never reach Caper's session. Taps on web links open the system browser
/// and never navigate in place.
private struct KlipyAdWebView: UIViewRepresentable {
    let html: String

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.allowsInlineMediaPlayback = true
        configuration.mediaTypesRequiringUserActionForPlayback = .all
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.navigationDelegate = context.coordinator
        view.uiDelegate = context.coordinator
        view.isOpaque = false
        view.backgroundColor = .clear
        view.scrollView.backgroundColor = .clear
        view.scrollView.isScrollEnabled = false
        view.scrollView.bounces = false
        let tap = UITapGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.tapped))
        tap.cancelsTouchesInView = false
        tap.delegate = context.coordinator
        view.addGestureRecognizer(tap)
        context.coordinator.html = html
        view.loadHTMLString(Self.document(html), baseURL: nil)
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {
        guard context.coordinator.html != html else { return }
        context.coordinator.html = html
        view.loadHTMLString(Self.document(html), baseURL: nil)
    }

    static func dismantleUIView(_ view: WKWebView, coordinator: Coordinator) {
        view.stopLoading()
        view.navigationDelegate = nil
        view.uiDelegate = nil
    }

    private static func document(_ html: String) -> String {
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">"
            + "<style>html,body{margin:0;padding:0;background:transparent;overflow:hidden}</style></head><body>"
            + html + "</body></html>"
    }

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate, UIGestureRecognizerDelegate {
        var html = ""
        private var lastTap = Date.distantPast

        @objc func tapped() { lastTap = Date() }

        func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }

        func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction) async -> WKNavigationActionPolicy {
            guard let url = navigationAction.request.url else { return .cancel }
            // The ad's own document and the frames inside it load in place.
            if navigationAction.targetFrame?.isMainFrame == false { return .allow }
            if url.scheme?.lowercased() == "about" { return .allow }
            // Anything else would leave the ad: only a tap may open it, in the system browser.
            let tapped = navigationAction.navigationType == .linkActivated || Date().timeIntervalSince(lastTap) < 1.5
            if tapped { open(url) }
            return .cancel
        }

        func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                     for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
            // Pop-ups need a user gesture by default; they open in the system browser.
            if let url = navigationAction.request.url { open(url) }
            return nil
        }

        private func open(_ url: URL) {
            guard let scheme = url.scheme?.lowercased(), scheme == "https" || scheme == "http" else { return }
            lastTap = .distantPast
            UIApplication.shared.open(url)
        }
    }
}
#endif
