import SwiftUI
#if os(iOS)
import UIKit
typealias PlatformViewController = UIViewController
#else
import AppKit
typealias PlatformViewController = NSViewController
#endif

/// The share sheet's "Caper" (docs/media.md, "Sharing into Caper"): your recent
/// channels and DMs, searchable. Picking one copies what was shared into the App
/// Group and opens Caper, which stages it in that conversation's composer. The
/// extension never holds your session or uploads anything itself.
@MainActor final class ShareState: ObservableObject {
    let recents = ShareInbox.recents()
    @Published var search = ""
    @Published var sending = false
    @Published var error: String?

    var unavailable: String? {
        if ShareInbox.root == nil { return "Sharing needs the Caper app from TestFlight or caper.chat." }
        if recents == nil { return "Open Caper and sign in, then share again." }
        return nil
    }
}

struct ShareView: View {
    @ObservedObject var state: ShareState
    let pick: (ShareDestination) -> Void
    let cancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Share to Caper").font(.headline).accessibilityAddTraits(.isHeader)
                Spacer()
                Button("Cancel", action: cancel).keyboardShortcut(.cancelAction)
            }
            if let message = state.unavailable {
                Text(message).foregroundStyle(.secondary)
                Spacer()
            } else {
                TextField("Find a channel or DM", text: $state.search)
                    .textFieldStyle(.roundedBorder)
                    .disabled(state.sending)
                let visible = ShareInbox.matching(state.recents?.destinations ?? [], search: state.search)
                if state.search.isEmpty, !visible.isEmpty { Text("Recent").font(.caption.bold()).foregroundStyle(.secondary) }
                if visible.isEmpty {
                    Text(state.search.isEmpty ? "No conversations yet. Join a channel or start a DM in Caper." : "No matching conversations.")
                        .foregroundStyle(.secondary)
                    Spacer()
                } else {
                    List(visible, id: \.id) { destination in
                        Button { pick(destination) } label: {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(destination.title).bold().lineLimit(1)
                                Text(destination.spaceName).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(state.sending)
                    }
                    .listStyle(.plain)
                }
            }
            if state.sending { Text("Opening Caper…").font(.caption).foregroundStyle(.secondary) }
            if let error = state.error { Text(error).font(.caption).foregroundStyle(.red) }
        }
        .padding(18)
        .preferredColorScheme(.dark)
        #if os(macOS)
        .frame(width: 360, height: 440)
        #endif
    }
}

final class ShareViewController: PlatformViewController {
    private let state = ShareState()

    private var shareView: ShareView {
        ShareView(state: state, pick: { [weak self] in self?.pick($0) }, cancel: { [weak self] in self?.cancel() })
    }

    #if os(iOS)
    override func viewDidLoad() {
        super.viewDidLoad()
        let host = UIHostingController(rootView: shareView)
        addChild(host)
        host.view.frame = view.bounds
        host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(host.view)
        host.didMove(toParent: self)
    }
    #else
    override func loadView() {
        view = NSHostingView(rootView: shareView)
        preferredContentSize = NSSize(width: 360, height: 440)
    }
    #endif

    private func cancel() {
        extensionContext?.cancelRequest(withError: NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError))
    }

    private func pick(_ destination: ShareDestination) {
        guard let recents = state.recents, !state.sending else { return }
        state.sending = true
        state.error = nil
        let items = extensionContext?.inputItems.compactMap { $0 as? NSExtensionItem } ?? []
        Task { @MainActor in
            do {
                let folder = try ShareInbox.newShareFolder()
                let collected = await ShareCollector.collect(items, into: folder)
                guard !collected.files.isEmpty || !collected.text.isEmpty else {
                    ShareInbox.remove(folder)
                    state.sending = false
                    state.error = "Nothing here can be shared to Caper."
                    return
                }
                try ShareInbox.complete(SharedItems(accountId: recents.accountId, destination: destination, text: collected.text,
                                                    files: collected.files, createdAt: Date()), in: folder)
                openCaper()
                extensionContext?.completeRequest(returningItems: nil)
            } catch {
                state.sending = false
                state.error = "Couldn’t hand this to Caper. Try again."
            }
        }
    }

    /// Opens `caper://share`. If the system declines, Caper still picks the
    /// share up the next time it opens.
    private func openCaper() {
        guard let url = URL(string: "\(ShareInbox.urlScheme)://\(ShareInbox.urlHost)") else { return }
        #if os(iOS)
        // Share extensions can't open URLs through their context; the app is on the responder chain.
        var responder: UIResponder? = self
        while let current = responder {
            if let application = current as? UIApplication {
                application.open(url, options: [:], completionHandler: nil)
                return
            }
            responder = current.next
        }
        #else
        NSWorkspace.shared.open(url)
        #endif
    }
}
