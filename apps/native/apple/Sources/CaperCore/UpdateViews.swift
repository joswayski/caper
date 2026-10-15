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
    public let version: String
    public let notes: String
    public let changelog: [ChangelogEntry]
    public let historyComplete: Bool
    public let canApply: Bool
    public let lastError: String?

    public init(version: String, notes: String, changelog: [ChangelogEntry],
                historyComplete: Bool, canApply: Bool, lastError: String?) {
        self.version = version; self.notes = notes; self.changelog = changelog
        self.historyComplete = historyComplete; self.canApply = canApply; self.lastError = lastError
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

/// Fixed actions and warning outside the independently scrolling release history.
@MainActor public struct DesktopUpdateReview: View {
    public let update: DesktopUpdate
    public let inCall: Bool
    public let close: () -> Void
    public let restart: () -> Void
    public let download: () -> Void
    @FocusState private var focused: Bool

    public init(update: DesktopUpdate, inCall: Bool, close: @escaping () -> Void,
                restart: @escaping () -> Void, download: @escaping () -> Void) {
        self.update = update; self.inCall = inCall; self.close = close
        self.restart = restart; self.download = download
    }

    public var body: some View {
        GeometryReader { geometry in
            ZStack {
                CaperTheme.blackout.opacity(0.86).ignoresSafeArea()
                    .onTapGesture(perform: close).accessibilityHidden(true)
                    .modifier(Entrance.scrim)
                VStack(alignment: .leading, spacing: 16) {
                    HStack(spacing: 14) {
                        UpdateCharacter().frame(width: 40, height: 40).accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 5) {
                            Text("A fresh Caper is ready").font(CaperTheme.font(20, weight: .bold))
                            Text("Caper \(update.version)").font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                        }
                        Spacer(minLength: 0)
                        Button(action: close) { CaperIcon(name: "x", size: 18) }
                            .buttonStyle(.plain).accessibilityLabel("Close update review")
                    }
                    Text("What’s new").font(CaperTheme.font(18, weight: .bold))
                    ScrollView {
                        VStack(alignment: .leading, spacing: 22) {
                            if let error = update.lastError {
                                VStack(alignment: .leading, spacing: 8) {
                                    Text("The last update didn’t finish").font(CaperTheme.font(14, weight: .bold))
                                    Text(error).textSelection(.enabled)
                                }.foregroundStyle(CaperTheme.error)
                            }
                            Text(update.historyNotice).foregroundStyle(CaperTheme.muted)
                            ForEach(Array(update.entries.enumerated()), id: \.offset) { _, entry in
                                VStack(alignment: .leading, spacing: 10) {
                                    Text(entry.version).font(CaperTheme.font(14, weight: .bold))
                                    Text(entry.notes.isEmpty ? "Release notes aren’t available for this version." : entry.notes)
                                        .lineSpacing(5).textSelection(.enabled)
                                }.frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }.font(CaperTheme.font(14)).frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.trailing, 12)
                    }.frame(maxWidth: .infinity, maxHeight: .infinity)
                    Divider().overlay(CaperTheme.border)
                    Text(update.canApply
                         ? (inCall ? "Restarting installs the update and leaves your current voice call." : "Caper restarts to install the update. Restarting leaves any voice call.")
                         : "This copy can’t update in place. Download the installer and drag Caper into a writable Applications folder.")
                        .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    HStack(spacing: 10) {
                        Button("Later", action: close).buttonStyle(CaperSecondaryButton())
                        Spacer()
                        Button(update.canApply ? (inCall ? "Restart and leave call" : "Restart to update") : "Download",
                               action: update.canApply ? restart : download).buttonStyle(CaperPrimaryButton())
                            .accessibilityIdentifier("desktop-update-install")
                    }
                    if update.canApply {
                        Button("Download installer instead", action: download)
                            .buttonStyle(.plain).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
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

private struct UpdateCharacter: View {
    var wave: Double = 0
    var bounce: Double = 0
    var body: some View {
        ZStack {
            Image("caper-update-feet", bundle: caperResourceBundle).renderingMode(.original).resizable()
                .rotationEffect(.radians(bounce * 0.06), anchor: UnitPoint(x: 0.5, y: 0.84))
                .offset(y: bounce)
            Image("caper-update-wave", bundle: caperResourceBundle).renderingMode(.original).resizable()
                .rotationEffect(.radians(wave), anchor: UnitPoint(x: 0.8, y: 0.66))
                .offset(y: bounce)
            Image("caper-update-body", bundle: caperResourceBundle).renderingMode(.original).resizable()
                .offset(y: bounce)
        }.aspectRatio(1, contentMode: .fit)
    }
}

#endif
