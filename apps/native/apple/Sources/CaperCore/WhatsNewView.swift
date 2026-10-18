import SwiftUI
import Observation
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

struct WhatsNewDisplay: Identifiable {
    var id = UUID()
    let sequence: Int64?
    let releases: [PlatformRelease]?
    let automatic: Bool
}

@MainActor @Observable public final class WhatsNewModel {
    private let api: APIClient
    fileprivate let revision: String
    private let platform: String
    private let history: WhatsNewHistory
    private(set) var loading = false
    private(set) var error: String?
    var workspaceModalOpen = false
    var settingsOpen = false
    var presentation: WhatsNewDisplay?

    public init(api: APIClient, defaults: UserDefaults = .standard) {
        self.api = api
        revision = Bundle.main.object(forInfoDictionaryKey: "CaperSourceRevision") as? String ?? ""
        #if os(iOS)
        platform = "ios"
        #else
        platform = "macos"
        #endif
        let key = "caper.whatsNew.\(APIClient.sessionService(for: api.baseURL)).\(platform)"
        history = WhatsNewHistory(read: {
            (defaults.object(forKey: key) as? NSNumber)?.int64Value
        }, write: { defaults.set(max((defaults.object(forKey: key) as? NSNumber)?.int64Value ?? 0, $0), forKey: key) })
    }

    func refresh() async {
        guard !loading else { return }
        guard WhatsNewSnapshot.validRevision(revision) else {
            error = "Release notes are unavailable in development builds."
            return
        }
        loading = true; error = nil
        defer { loading = false }
        do {
            let snapshot = try await api.whatsNew(platform: platform, revision: revision)
            try history.receive(snapshot, platform: platform, revision: revision)
            if let shown = presentation, shown.releases == nil {
                presentation = WhatsNewDisplay(id: shown.id, sequence: snapshot.sequence, releases: history.retained, automatic: false)
            }
        } catch is CancellationError { }
        catch { self.error = "Could not load release notes. Please try again." }
    }

    var hasSnapshot: Bool { history.snapshot != nil }
    func open() {
        presentation = WhatsNewDisplay(sequence: history.snapshot?.sequence, releases: hasSnapshot ? history.retained : nil, automatic: false)
        Task { await refresh() }
    }
    func presentAutomatically() {
        guard presentation == nil, !history.unread.isEmpty else { return }
        presentation = WhatsNewDisplay(sequence: history.snapshot?.sequence, releases: history.unread, automatic: true)
    }
    /// The desktop update review already showed this build's highlights;
    /// installing from it marks them read so the new build does not repeat them.
    public func markRead(_ sequence: Int64) {
        guard (1...WhatsNewSnapshot.maximumSequence).contains(sequence) else { return }
        history.acknowledge(sequence)
    }

    func dismiss(_ display: WhatsNewDisplay) {
        if let sequence = display.sequence { history.acknowledge(sequence) }
        if presentation?.id == display.id { presentation = nil }
    }

    /// Custom workspace overlays are tracked separately; native presentations include
    /// nested message sheets, popovers, permissions and the desktop updater's alerts.
    static var nativeModalOpen: Bool {
        #if os(iOS)
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows)
            .contains { $0.rootViewController?.presentedViewController != nil }
        #else
        NSApp.modalWindow != nil || NSApp.windows.contains { $0.attachedSheet != nil }
        #endif
    }
}

@MainActor struct WhatsNewPresentation: ViewModifier {
    @Bindable var model: AppModel
    @Environment(\.scenePhase) private var scenePhase
    private var ready: Bool { model.phase == .ready && model.account != nil && model.spacesLoaded && !model.restoringWorkspace }

    func body(content: Content) -> some View {
        content
            .sheet(item: Binding(get: { model.whatsNew.settingsOpen ? nil : model.whatsNew.presentation }, set: { model.whatsNew.presentation = $0 })) { display in
                WhatsNewSheet(notes: model.whatsNew, display: model.whatsNew.presentation ?? display)
            }
            .task(id: "\(ready)-\(scenePhase == .active)") {
                guard ready, scenePhase == .active else { return }
                repeat {
                    await model.whatsNew.refresh()
                    if model.whatsNew.hasSnapshot { break }
                    do { try await Task.sleep(for: .seconds(60)) } catch { return }
                } while !Task.isCancelled
            }
            .task(id: "presentation-\(ready)-\(scenePhase == .active)") {
                guard ready, scenePhase == .active else { return }
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .milliseconds(500)) } catch { return }
                    let notes = model.whatsNew
                    guard model.voice.phase == .idle || model.voice.phase == .failed else { continue }
                    if !notes.workspaceModalOpen && !notes.settingsOpen && !WhatsNewModel.nativeModalOpen {
                        notes.presentAutomatically()
                    }
                }
            }
    }
}

/// A muted release-source link with pointer, press, and keyboard feedback.
/// Used wherever a version is shown.
struct ReleaseLink: View {
    let label: String
    let url: URL
    var size: CGFloat = 13

    var body: some View {
        Link(destination: url) {
            Text(label).font(CaperTheme.font(size))
        }
        .buttonStyle(ReleaseLinkStyle())
        .help("View this version on GitHub")
        .accessibilityHint("View this version on GitHub")
    }
}

/// Keep commit links quiet at rest without inheriting the sheet's filled button style.
private struct ReleaseLinkStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.underline()
            .modifier(ReleaseLinkFeedback(pressed: configuration.isPressed))
    }
}

private struct ReleaseLinkFeedback: ViewModifier {
    let pressed: Bool
    @Environment(\.isFocused) private var focused
    @State private var hovered = false

    func body(content: Content) -> some View {
        content.foregroundStyle(pressed ? CaperTheme.terracottaBright : hovered ? CaperTheme.actionHover : CaperTheme.muted)
            .background(CaperTheme.terracotta.opacity(pressed ? 0.24 : hovered ? 0.12 : 0))
            .overlay {
                RoundedRectangle(cornerRadius: 2)
                    .stroke(CaperTheme.terracotta, lineWidth: 2).padding(-3)
                    .opacity(focused ? 1 : 0).allowsHitTesting(false)
            }
            .focusEffectDisabled()
            .onHover { hovered = $0 }
            .modifier(ControlPointer())
    }
}

/// "October 18, 2026" for a release's UTC publication day ("2026-10-18").
func releaseDayLabel(_ day: String) -> String {
    let parser = DateFormatter()
    parser.locale = Locale(identifier: "en_US_POSIX")
    parser.timeZone = TimeZone(identifier: "UTC")
    parser.dateFormat = "yyyy-MM-dd"
    guard let date = parser.date(from: day) else { return day }
    parser.dateFormat = "MMMM d, yyyy"
    return parser.string(from: date)
}

@MainActor struct WhatsNewSheet: View {
    @Bindable var notes: WhatsNewModel
    @State private var display: WhatsNewDisplay

    init(notes: WhatsNewModel, display: WhatsNewDisplay) {
        self.notes = notes
        _display = State(initialValue: display)
    }

    private func close() { notes.dismiss(display) }
    private var installedVersion: String {
        (Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String)
            .map { "Version \($0)" } ?? "Development build"
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // Like the update review: the face, the title, then the installed
            // version right under it, linked to its commit.
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 0) {
                    Image("caper-face", bundle: caperResourceBundle).renderingMode(.original).resizable()
                        .scaledToFit().frame(width: 36, height: 36).accessibilityHidden(true)
                        .padding(.bottom, 10)
                    Text("What’s new").font(CaperTheme.font(20, weight: .bold)).accessibilityAddTraits(.isHeader)
                        .padding(.bottom, 2)
                    if WhatsNewSnapshot.validRevision(notes.revision),
                       let url = URL(string: "https://github.com/joswayski/caper/commit/\(notes.revision)") {
                        ReleaseLink(label: installedVersion, url: url)
                    } else {
                        Text(installedVersion).font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                    }
                }
                Spacer()
                Button("Close", action: close).accessibilityLabel("Close What’s new")
            }.padding(22)
            Divider().overlay(CaperTheme.border)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let releases = display.releases {
                        if releases.isEmpty {
                            Text("No release highlights for this version yet.").foregroundStyle(CaperTheme.muted)
                        }
                        let days = Dictionary(grouping: releases) { String($0.publishedAt.prefix(10)) }
                        ForEach(days.keys.sorted(by: >), id: \.self) { date in
                            Text(releaseDayLabel(date)).font(CaperTheme.font(12, weight: .bold))
                                .foregroundStyle(CaperTheme.muted)
                            ForEach(days[date] ?? [], id: \.sequence) { release in
                                VStack(alignment: .leading, spacing: 12) {
                                    if let url = URL(string: "https://github.com/joswayski/caper/commit/\(release.revision)") {
                                        ReleaseLink(
                                            label: release.version.map { "Version \($0)" } ?? "Commit \(release.revision.prefix(7))",
                                            url: url, size: 12)
                                    }
                                    ForEach(Array(release.highlights.enumerated()), id: \.offset) { _, highlight in
                                        Text(highlight.title).font(CaperTheme.font(16, weight: .bold))
                                        Text(highlight.body).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted)
                                    }
                                }
                            }
                            Divider().overlay(CaperTheme.border)
                        }
                    } else if notes.loading {
                        ProgressView().accessibilityLabel("Loading release highlights")
                        Text("Loading release highlights…").foregroundStyle(CaperTheme.muted)
                    } else if let error = notes.error {
                        Text(error).foregroundStyle(CaperTheme.error)
                        Button("Retry") { Task { await notes.refresh() } }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading).padding(22)
            }
            HStack {
                Spacer()
                Button(display.automatic ? "Got it" : "Done", action: close).buttonStyle(CaperPrimaryButton())
            }.padding(22)
        }
        .font(CaperTheme.font(14)).foregroundStyle(CaperTheme.text)
        .background(CaperTheme.surface).preferredColorScheme(.dark).tint(CaperTheme.terracottaBright)
        .buttonStyle(CaperSecondaryButton()).accessibilityIdentifier("whats-new")
        .onChange(of: notes.presentation?.sequence, initial: true) { _, _ in
            if display.releases == nil, let loaded = notes.presentation, loaded.id == display.id {
                display = loaded
            }
        }
        .onDisappear { notes.dismiss(display) }
        #if os(macOS)
        .frame(width: 520, height: 540).onExitCommand(perform: close)
        #else
        .presentationDetents([.large]).presentationDragIndicator(.visible)
        #endif
    }
}
