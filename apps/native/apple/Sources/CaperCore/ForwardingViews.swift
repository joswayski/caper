import SwiftUI

struct SharedOriginalView: View {
    let message: ChatMessage
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Avatar(name: message.author.name, size: 24, avatarID: message.author.avatarId)
                Text(message.author.name).font(CaperTheme.font(13, weight: .bold))
                if message.editedAt != nil { Text("edited").font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted) }
            }
            // File-only messages carry empty text; hide the empty row.
            if !message.content.text.isEmpty {
                // Links open in the system browser; selection would take their clicks on macOS.
                if let linked = MessageLinks.attributed(message.content.text) {
                    Text(linked).font(CaperTheme.font(14)).tint(CaperTheme.terracottaBright)
                } else {
                    Text(message.content.text).font(CaperTheme.font(14)).textSelection(.enabled)
                }
            }
            if let attachments = message.content.attachments, !attachments.isEmpty {
                // Another conversation's files: shown as delivered, without
                // this timeline's URL refresh or upload progress.
                MessageAttachmentsView(attachments: attachments, chat: nil)
            }
            ReactionFlowLayout(spacing: 8) {
                ForEach(message.reactions ?? []) { reaction in
                    HStack(spacing: 4) {
                        EmojiArtworkView(emoji: reaction.emoji, size: 18)
                        Text("\(reaction.authorIds.count)").font(CaperTheme.font(12))
                    }.accessibilityLabel("\(reaction.emoji), \(reaction.authorIds.count) reactions")
                }
            }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct ForwardCardView: View {
    let message: ChatMessage
    let open: () -> Void
    var body: some View {
        if let forward = message.forward {
            VStack(alignment: .leading, spacing: 8) {
                Label("Forwarded · live", systemImage: "arrowshape.turn.up.right").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                if let original = forward.message {
                    SharedOriginalView(message: original)
                    Button("\(original.thread.map { "\($0.replyCount) \($0.replyCount == 1 ? "reply" : "replies") · " } ?? "")View conversation", action: open)
                        .buttonStyle(CaperSecondaryButton())
                } else { Text("Original conversation unavailable.").foregroundStyle(CaperTheme.muted) }
            }.padding(12).background(CaperTheme.surface, in: RoundedRectangle(cornerRadius: 8))
                .overlay { RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border) }
        }
    }
}

struct ForwardPickerView: View {
    @Environment(\.dismiss) private var dismiss
    let chat: ChatModel
    let message: ChatMessage
    @State private var destinations: [ForwardDestination]?
    @State private var selected: Set<String> = []
    @State private var search = ""
    @State private var note = ""
    @State private var pending: [(destination: String, key: String)]?
    @State private var confirmed = 0
    @State private var sending = false
    @State private var sendTask: Task<Void, Never>?
    @State private var error: String?
    @State private var attempt = 0
    private var visible: [ForwardDestination] {
        let terms = search.split(whereSeparator: { $0.isWhitespace }).map { $0.hasPrefix("#") ? String($0.dropFirst()) : String($0) }
        return (destinations ?? []).filter { destination in
            terms.allSatisfy { "\(destination.spaceName) \(destination.name)".localizedCaseInsensitiveContains($0) || $0.isEmpty }
        }
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("Forward message").font(CaperTheme.font(16, weight: .bold)); Spacer(); Button("Close") { dismiss() } }.padding(16)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    Text("Shares this conversation live, including future edits, reactions and replies. People in the destination can read and forward it.")
                        .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    SharedOriginalView(message: message.forward?.message ?? message)
                    TextField("Find a space, channel or DM", text: $search).textFieldStyle(.roundedBorder).disabled(pending != nil)
                    ForEach(visible) { destination in
                        Button {
                            if selected.contains(destination.id) { selected.remove(destination.id) }
                            else { selected.insert(destination.id) }
                        } label: {
                            HStack(spacing: 12) {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text("\(destination.direct ? "" : "# ")\(destination.name)").fontWeight(.bold)
                                    Text(destination.spaceName).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                                }
                                Spacer()
                                Image(systemName: selected.contains(destination.id) ? "checkmark.square.fill" : "square")
                                    .foregroundStyle(selected.contains(destination.id) ? CaperTheme.terracottaBright : CaperTheme.muted)
                            }.frame(minHeight: 44)
                                // The whole row toggles, including the gap before the checkbox.
                                .contentShape(Rectangle())
                        }.buttonStyle(.plain).disabled(pending != nil)
                            .accessibilityValue(selected.contains(destination.id) ? "Selected" : "Not selected")
                            .modifier(ControlPointer())
                    }
                    if destinations == nil && error == nil { ProgressView("Loading destinations…") }
                    if destinations?.isEmpty == true { Text("Join a channel or start a DM to forward here.") }
                    else if destinations != nil && visible.isEmpty { Text("No matching destinations.") }
                    TextField("Add a note (optional)", text: $note, axis: .vertical).lineLimit(2...5).textFieldStyle(.roundedBorder).disabled(pending != nil)
                    if let error {
                        Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.terracottaBright)
                        if destinations == nil { Button("Retry loading") { attempt += 1 } }
                    }
                    Button(sending ? "Forwarding…" : pending == nil ? "Forward (\(selected.count))" : "Retry forwards (\(selected.count))") {
                        guard !selected.isEmpty, !sending else { return }
                        let intent = pending ?? (destinations ?? []).filter { selected.contains($0.id) }.map { (destination: $0.id, key: UUID().uuidString.lowercased()) }
                        let text = note.trimmingCharacters(in: .whitespacesAndNewlines)
                        pending = intent; sending = true; error = nil
                        sendTask = Task { @MainActor in
                            defer { sending = false }
                            do {
                                for next in intent {
                                    try Task.checkCancellation()
                                    try await chat.forward(message: message, destinationID: next.destination, key: next.key, text: text)
                                    confirmed += 1
                                    selected.remove(next.destination)
                                    pending?.removeFirst()
                                }
                                dismiss()
                            } catch is CancellationError { }
                            catch {
                                let rejected = (error as? APIError).map { [400, 401, 403, 404, 409, 422].contains($0.status) } ?? false
                                if rejected { pending = nil }
                                self.error = "\(confirmed > 0 ? "Forwarded to \(confirmed) \(confirmed == 1 ? "destination" : "destinations"). " : "")\(rejected ? "Remaining forwards not sent." : "Remaining forwards not confirmed. Retry checks the same forwards.") \(error.localizedDescription)"
                            }
                        }
                    }.buttonStyle(CaperSecondaryButton()).disabled(selected.isEmpty || sending || note.unicodeScalars.count > 4000)
                }.padding(16)
            }
        }.background(CaperTheme.raised).foregroundStyle(CaperTheme.text)
            .frame(idealWidth: 460, idealHeight: 560)
            // A sheet: only it can present the viewer over itself.
            .modifier(MediaViewerHost())
            .onDisappear { sendTask?.cancel() }
            .task(id: attempt) {
                do { destinations = try await chat.forwardDestinations().sorted { "\($0.spaceName) \($0.name)" < "\($1.spaceName) \($1.name)" }; error = nil }
                catch is CancellationError { }
                catch { self.error = error.localizedDescription }
            }
    }
}

struct ForwardConversationView: View {
    @Environment(\.dismiss) private var dismiss
    let chat: ChatModel
    let messageID: String
    @State private var conversation: ForwardConversationHistory?
    @State private var oldest: String?
    @State private var pages = 1
    @State private var loading = false
    @State private var error: String?
    @State private var attempt = 0
    private var message: ChatMessage? { (chat.messages + chat.pinnedMessages).first { $0.id == messageID } }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("Forwarded conversation").font(CaperTheme.font(16, weight: .bold)); Spacer(); Button("Close") { dismiss() } }.padding(16)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text("Live · Read-only original. Replies to the forward stay in the destination.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    if let root = conversation?.root {
                        SharedOriginalView(message: root)
                        let count = root.thread?.replyCount ?? conversation?.messages.count ?? 0
                        Text("\(count) \(count == 1 ? "reply" : "replies")").font(CaperTheme.font(13, weight: .bold))
                        if conversation?.hasMore == true { Button("Load older replies") { pages += 1 }.disabled(loading) }
                        ForEach(conversation?.messages ?? []) { SharedOriginalView(message: $0) }
                        if conversation?.messages.isEmpty == true { Text("No replies yet.") }
                    } else if !loading && error == nil { Text("Original conversation unavailable.") }
                    if loading { ProgressView("Updating conversation…") }
                    if let error {
                        Text(error).foregroundStyle(CaperTheme.terracottaBright)
                        Button("Retry") { attempt += 1 }
                    }
                }.padding(16).frame(maxWidth: .infinity, alignment: .leading)
            }
        }.background(CaperTheme.raised).foregroundStyle(CaperTheme.text).frame(idealWidth: 520, idealHeight: 560)
            // A sheet: only it can present the viewer over itself.
            .modifier(MediaViewerHost())
            .task(id: "\(message?.forward?.seq ?? "0"):\(pages):\(attempt)") {
                guard let message else { conversation = nil; return }
                loading = true; error = nil
                defer { loading = false }
                do {
                    var fresh = try await chat.forwardedConversation(message: message)
                    var loaded = 1
                    while fresh.hasMore, let first = fresh.messages.first,
                          loaded < pages || oldest.map({ (try? Sequence.compare(first.seq, $0)) == .orderedDescending }) == true {
                        let earlier = try await chat.forwardedConversation(message: message, before: first.seq)
                        if earlier.messages.isEmpty { fresh.hasMore = false; break }
                        fresh.messages = earlier.messages + fresh.messages; fresh.hasMore = earlier.hasMore; loaded += 1
                    }
                    try Task.checkCancellation()
                    oldest = fresh.messages.first?.seq; conversation = fresh
                } catch is CancellationError { }
                catch { conversation = nil; self.error = error.localizedDescription }
            }
    }
}
