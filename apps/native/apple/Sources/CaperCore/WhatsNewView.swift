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
    private let revision: String
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

@MainActor struct WhatsNewSheet: View {
    @Bindable var notes: WhatsNewModel
    @State private var display: WhatsNewDisplay

    init(notes: WhatsNewModel, display: WhatsNewDisplay) {
        self.notes = notes
        _display = State(initialValue: display)
    }

    private func close() { notes.dismiss(display) }
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 6) {
                    Text("What’s new").font(CaperTheme.font(20, weight: .bold)).accessibilityAddTraits(.isHeader)
                    Text("Updates to Caper on \(platformName).")
                        .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
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
                        ForEach(releases, id: \.sequence) { release in
                            Text(String(release.publishedAt.prefix(10))).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                            ForEach(Array(release.highlights.enumerated()), id: \.offset) { _, highlight in
                                Text(highlight.title).font(CaperTheme.font(16, weight: .bold))
                                Text(highlight.body).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted)
                            }
                            Divider().overlay(CaperTheme.border)
                        }
                    } else if notes.loading {
                        ProgressView().accessibilityLabel("Loading release notes")
                        Text("Loading release notes…").foregroundStyle(CaperTheme.muted)
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
    private var platformName: String {
        #if os(iOS)
        "iOS"
        #else
        "macOS"
        #endif
    }
}
