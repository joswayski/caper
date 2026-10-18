import SwiftUI

#if os(macOS)
import AppKit

/// The updater's review payload, shared directly with the window presentation.
public struct DesktopUpdate: Equatable, Sendable {
    public struct ChangelogEntry: Decodable, Equatable, Sendable {
        public let version: String
        public let notes: String
        public init(version: String, notes: String) { self.version = version; self.notes = notes }
    }
    public struct Highlight: Decodable, Equatable, Sendable {
        public let title: String
        public let body: String
        public init(title: String, body: String) { self.title = title; self.body = body }
    }
    public let version: String
    public let notes: String
    public let changelog: [ChangelogEntry]
    public let historyComplete: Bool
    public let canApply: Bool
    public let lastError: String?
    /// Every release being installed, stacked newest first. `nil` when the
    /// highlights could not be read, so the plain notes explain why.
    public let highlights: [Highlight]?
    /// The newest highlighted release's date (YYYY-MM-DD).
    public let released: String?
    /// The update's source commit, linked from its version.
    public let revision: String?
    /// The update's highlights sequence, marked read when it is installed.
    public let sequence: Int64?

    public init(version: String, notes: String, changelog: [ChangelogEntry],
                historyComplete: Bool, canApply: Bool, lastError: String?,
                highlights: [Highlight]? = nil, released: String? = nil,
                revision: String? = nil, sequence: Int64? = nil) {
        self.version = version; self.notes = notes; self.changelog = changelog
        self.historyComplete = historyComplete; self.canApply = canApply; self.lastError = lastError
        self.highlights = highlights; self.released = released
        self.revision = revision; self.sequence = sequence
    }

    /// This version's commit on GitHub, only for a full source SHA.
    public var commitURL: URL? {
        guard let revision, revision.count == 40,
              revision.allSatisfy({ $0.isHexDigit && !$0.isUppercase }) else { return nil }
        return URL(string: "https://github.com/joswayski/caper/commit/\(revision)")
    }

    /// People know when they updated, not which build: "October 18, 2026".
    public var releasedLabel: String? {
        guard let released else { return nil }
        let parser = DateFormatter()
        parser.locale = Locale(identifier: "en_US_POSIX")
        parser.timeZone = TimeZone(identifier: "UTC")
        parser.dateFormat = "yyyy-MM-dd"
        guard let date = parser.date(from: released) else { return nil }
        let formatter = DateFormatter()
        formatter.timeZone = TimeZone(identifier: "UTC")
        formatter.dateStyle = .long
        formatter.timeStyle = .none
        return formatter.string(from: date)
    }

    public var entries: [ChangelogEntry] {
        // The signed manifest already orders history by release build, not version text.
        changelog.isEmpty ? [ChangelogEntry(version: version, notes: notes)] : changelog
    }

    public var historyNotice: String {
        historyComplete ? "This update includes all of the following changes."
            : "Recorded changes are shown below. Earlier release notes aren’t available."
    }
}

/// Decoration only: the existing button owns layout, focus and activation.
/// A short shine every eight seconds; background/Reduce Motion keeps the edge still.
struct UpdateButtonShine: ViewModifier {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.controlActiveState) private var activity
    @Environment(\.isEnabled) private var enabled
    @State private var progress = 0.0
    private var running: Bool { enabled && !reduceMotion && activity == .key }
    private let duration = CaperMotion.slow * 6
    private let colors = [
        Color(red: 167/255, green: 128/255, blue: 242/255),
        Color(red: 104/255, green: 175/255, blue: 236/255),
        Color(red: 99/255, green: 210/255, blue: 193/255),
        Color(red: 215/255, green: 205/255, blue: 130/255),
        Color(red: 229/255, green: 139/255, blue: 183/255),
    ]

    func body(content: Content) -> some View {
        content.overlay {
            ZStack {
                let edge = LinearGradient(colors: colors, startPoint: .leading, endPoint: .trailing)
                RoundedRectangle(cornerRadius: 8).strokeBorder(edge.opacity(0.12), lineWidth: 4)
                RoundedRectangle(cornerRadius: 8).strokeBorder(edge.opacity(0.75), lineWidth: 1)
                UpdateButtonSweep(progress: progress)
                    .opacity(running ? 1 : 0)
            }.opacity(enabled ? 1 : 0)
                .allowsHitTesting(false).accessibilityHidden(true)
        }
        .task(id: running) {
            withAnimation(nil) { progress = 0 }
            guard running else { return }
            while !Task.isCancelled {
                withAnimation(CaperMotion.easeOut(duration)) { progress = 1 }
                do { try await Task.sleep(for: .seconds(duration)) } catch { return }
                withAnimation(nil) { progress = 0 }
                do { try await Task.sleep(for: .seconds(8 - duration)) } catch { return }
            }
        }
    }
}

/// Animate drawing coordinates, not an oversized child view that expands the button's AX frame.
private struct UpdateButtonSweep: View, Animatable {
    var progress: Double
    var animatableData: Double {
        get { progress }
        set { progress = newValue }
    }

    var body: some View {
        Canvas { context, size in
            context.clip(to: Path(roundedRect: CGRect(origin: .zero, size: size), cornerRadius: 8))
            context.translateBy(x: -80 + (size.width + 160) * progress, y: -size.height)
            context.rotate(by: .degrees(18))
            context.fill(Path(CGRect(x: 0, y: 0, width: 56, height: size.height * 3)),
                         with: .linearGradient(Gradient(colors: [.clear, .white.opacity(0.16), .clear]),
                                               startPoint: .zero, endPoint: CGPoint(x: 56, y: 0)))
        }
    }
}

/// Fixed actions and warning outside the independently scrolling release history.
@MainActor public struct DesktopUpdateReview: View {
    public let update: DesktopUpdate
    public let inCall: Bool
    public let close: () -> Void
    public let restart: () -> Void
    public let download: () -> Void
    /// Sends the error to the Caper team; returns a failure message or nil.
    public let report: ((String) async -> String?)?
    @FocusState private var focused: Bool
    @State private var reportState: ReportState?

    enum ReportState: Equatable { case sending, sent, failed(String) }

    public init(update: DesktopUpdate, inCall: Bool, close: @escaping () -> Void,
                restart: @escaping () -> Void, download: @escaping () -> Void,
                report: ((String) async -> String?)? = nil) {
        self.update = update; self.inCall = inCall; self.close = close
        self.restart = restart; self.download = download; self.report = report
    }

    public var body: some View {
        GeometryReader { geometry in
            ZStack {
                CaperTheme.blackout.opacity(0.86).ignoresSafeArea()
                    .onTapGesture(perform: close).accessibilityHidden(true)
                    .modifier(Entrance.scrim)
                VStack(alignment: .leading, spacing: 16) {
                    HStack(spacing: 14) {
                        Image("caper-face", bundle: caperResourceBundle).renderingMode(.original).resizable()
                            .scaledToFit().frame(width: 40, height: 40).accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 5) {
                            Text("A fresh Caper is ready").font(CaperTheme.font(20, weight: .bold))
                            // "Version 0.1.82 · October 18, 2026"; the version opens its commit.
                            HStack(spacing: 0) {
                                if let url = update.commitURL {
                                    Link("Version \(update.version)", destination: url)
                                        .help("View this version on GitHub")
                                } else {
                                    Text("Version \(update.version)")
                                }
                                if let date = update.releasedLabel { Text(" · \(date)") }
                            }.font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).tint(CaperTheme.muted)
                        }
                        Spacer(minLength: 0)
                        Button(action: close) { CaperIcon(name: "x", size: 18) }
                            .buttonStyle(.plain).accessibilityLabel("Close update review")
                    }
                    HStack(spacing: 10) {
                        Text("WHAT’S NEW").font(CaperTheme.font(11, weight: .bold)).tracking(0.8)
                            .foregroundStyle(CaperTheme.muted)
                        Rectangle().fill(CaperTheme.border).frame(height: 1)
                    }.accessibilityElement(children: .combine).accessibilityAddTraits(.isHeader)
                    ScrollView {
                        VStack(alignment: .leading, spacing: 16) {
                            if let error = update.lastError {
                                VStack(alignment: .leading, spacing: 8) {
                                    Text("The last update didn’t finish").font(CaperTheme.font(14, weight: .bold))
                                        .foregroundStyle(CaperTheme.error)
                                    // The installer link sits in the sentence; no separate footer link.
                                    // Literals, not a ternary: only a literal Text parses the markdown link.
                                    Group {
                                        if update.canApply {
                                            Text("Try again using the button below, or [use the installer](caper-update:installer).")
                                        } else {
                                            Text("Download the installer with the button below.")
                                        }
                                    }.tint(CaperTheme.text).foregroundStyle(CaperTheme.muted)
                                        .environment(\.openURL, OpenURLAction { _ in download(); return .handled })
                                    Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                                        .textSelection(.enabled)
                                    if let report {
                                        // One click posts this error to the Caper team from your notes.
                                        switch reportState {
                                        case .sent?:
                                            Text("Sent. The Caper team will reply in your notes.")
                                                .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                                        case .sending?:
                                            Button("Sending to the Caper team…") {}.buttonStyle(CaperSecondaryButton()).disabled(true)
                                        case .failed(let message)?:
                                            Text(message).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                                            reportButton(report, error: error)
                                        case nil:
                                            reportButton(report, error: error)
                                        }
                                    }
                                }
                            }
                            if !update.historyComplete {
                                Text("Earlier release notes aren’t available.")
                                    .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                            }
                            if let highlights = update.highlights {
                                if highlights.isEmpty {
                                    Text("Behind-the-scenes fixes this time.").foregroundStyle(CaperTheme.muted)
                                }
                                ForEach(Array(highlights.enumerated()), id: \.offset) { _, item in
                                    UpdateHighlightRow(title: item.title, text: item.body)
                                }
                            } else {
                                // Highlights could not be read: the updater's plain
                                // notes and the signed changelog, without versions.
                                Text(update.notes).foregroundStyle(CaperTheme.muted).textSelection(.enabled)
                                ForEach(Array(update.changelog.enumerated()), id: \.offset) { _, entry in
                                    Text(entry.notes).foregroundStyle(CaperTheme.muted)
                                        .lineSpacing(5).textSelection(.enabled)
                                }
                            }
                        }.font(CaperTheme.font(14)).frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.trailing, 12)
                    }.frame(maxWidth: .infinity, maxHeight: .infinity)
                    Divider().overlay(CaperTheme.border)
                    if !update.canApply {
                        Text("This copy can’t update in place. Download the installer and drag Caper into a writable Applications folder.")
                            .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    }
                    HStack(spacing: 10) {
                        Button("Later", action: close).buttonStyle(CaperSecondaryButton())
                        Spacer()
                        Button(update.canApply ? (inCall ? "Restart and leave call" : "Restart to update") : "Download",
                               action: update.canApply ? restart : download).buttonStyle(CaperPrimaryButton())
                            .accessibilityIdentifier("desktop-update-install")
                    }
                }
                .padding(20).frame(width: min(560, max(0, geometry.size.width - 32)),
                                   height: min(660, max(0, geometry.size.height - 32)))
                .background(CaperTheme.surface, in: RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                .modifier(Entrance(offset: CGSize(width: 0, height: 8), animation: CaperMotion.easeOut(CaperMotion.base)))
                .contentShape(Rectangle()).onTapGesture { }
            }.frame(width: geometry.size.width, height: geometry.size.height)
        }
        .foregroundStyle(CaperTheme.text).preferredColorScheme(.dark)
        .accessibilityElement(children: .contain).accessibilityLabel("Update review")
        .accessibilityAddTraits(.isModal).accessibilityIdentifier("desktop-update-review")
        .focusable().focusEffectDisabled().focused($focused)
        .onAppear { NSApp.keyWindow?.makeFirstResponder(nil); focused = true }
        .onExitCommand(perform: close)
    }
}

/// One release highlight: a bullet, a bold title and a muted description.
private struct UpdateHighlightRow: View {
    let title: String
    let text: String
    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Circle().fill(CaperTheme.muted).frame(width: 5, height: 5)
                .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(CaperTheme.font(15, weight: .bold)).foregroundStyle(CaperTheme.text)
                Text(text).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted).lineSpacing(3)
            }.textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
        }.accessibilityElement(children: .combine)
    }
}

extension DesktopUpdateReview {
    fileprivate func reportButton(_ report: @escaping (String) async -> String?, error: String) -> some View {
        Button("Send this to the Caper team") {
            reportState = .sending
            Task { @MainActor in
                reportState = await report(error).map { .failed($0) } ?? .sent
            }
        }.buttonStyle(CaperSecondaryButton()).accessibilityIdentifier("desktop-update-report")
    }
}

#endif
