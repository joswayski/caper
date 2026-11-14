import Foundation
import Observation

/// State owned by one native editor, never the channel or thread composer.
@MainActor @Observable final class NativeMessageEdit {
    let chat: ChatModel
    private(set) var baseline: ChatMessage
    var draft: String
    private(set) var busy = false
    private(set) var error: String?
    @ObservationIgnored private var task: Task<Void, Never>?
    @ObservationIgnored private let close: () -> Void

    init(chat: ChatModel, message: ChatMessage, close: @escaping () -> Void) {
        self.chat = chat; baseline = message; draft = message.content.text; self.close = close
    }
    var canSave: Bool { !busy && MessageValidation.error(for: draft) == nil && chat.canEdit(baseline) }
    func save() {
        guard canSave else { return }
        perform { [self] in
            try await chat.editMessage(baseline, text: draft)
            try Task.checkCancellation()
            close()
        }
    }
    func reload() {
        guard !busy else { return }
        perform { [self] in
            let latest = try await chat.reloadMessage(baseline)
            try Task.checkCancellation()
            baseline = latest; draft = latest.content.text
        }
    }
    private func perform(_ operation: @escaping @MainActor () async throws -> Void) {
        busy = true; error = nil
        task = Task { [weak self] in
            defer { self?.busy = false }
            do { try await operation() }
            catch is CancellationError { }
            catch { if !Task.isCancelled { self?.error = error.localizedDescription } }
        }
    }
    func stop() { task?.cancel(); task = nil }
}

/// One forwarding intent. An uncertain POST keeps the same key and note for
/// retry, while confirmed destinations are never sent again.
@MainActor @Observable final class NativeForwardPicker {
    let chat: ChatModel
    let message: ChatMessage
    private(set) var destinations: [ForwardDestination]?
    var search = ""
    var note = ""
    private(set) var selected: Set<String> = []
    private(set) var pending: [(destination: String, key: String)]?
    private(set) var confirmed = 0
    private(set) var sending = false
    private(set) var error: String?
    @ObservationIgnored private var task: Task<Void, Never>?
    @ObservationIgnored private let close: () -> Void

    init(chat: ChatModel, message: ChatMessage, close: @escaping () -> Void) {
        self.chat = chat; self.message = message; self.close = close
    }
    var visible: [ForwardDestination] {
        let terms = search.split(whereSeparator: { $0.isWhitespace }).map { $0.hasPrefix("#") ? String($0.dropFirst()) : String($0) }
        return (destinations ?? []).filter { destination in
            terms.allSatisfy { "\(destination.spaceName) \(destination.name)".localizedCaseInsensitiveContains($0) || $0.isEmpty }
        }
    }
    var editable: Bool { pending == nil }
    var canSend: Bool { !selected.isEmpty && !sending && note.unicodeScalars.count <= 4000 }
    var action: String { sending ? "Forwarding…" : pending == nil ? "Forward (\(selected.count))" : "Retry forwards (\(selected.count))" }
    func toggle(_ id: String) {
        guard editable else { return }
        if selected.contains(id) { selected.remove(id) } else { selected.insert(id) }
    }
    func load() {
        task?.cancel(); error = nil
        task = Task { [weak self, chat] in
            do {
                let destinations = try await chat.forwardDestinations().sorted { "\($0.spaceName) \($0.name)" < "\($1.spaceName) \($1.name)" }
                try Task.checkCancellation()
                self?.destinations = destinations
            } catch is CancellationError { }
            catch { if !Task.isCancelled { self?.error = error.localizedDescription } }
        }
    }
    func send() {
        guard canSend else { return }
        let intent = pending ?? (destinations ?? []).filter { selected.contains($0.id) }
            .map { (destination: $0.id, key: UUID().uuidString.lowercased()) }
        let text = note.trimmingCharacters(in: .whitespacesAndNewlines)
        pending = intent; sending = true; error = nil
        task = Task { [weak self, chat, message, close] in
            defer { self?.sending = false }
            do {
                for next in intent {
                    try Task.checkCancellation()
                    try await chat.forward(message: message, destinationID: next.destination, key: next.key, text: text)
                    try Task.checkCancellation()
                    self?.confirmed += 1
                    self?.selected.remove(next.destination)
                    self?.pending?.removeFirst()
                }
                close()
            } catch is CancellationError { }
            catch {
                guard let self, !Task.isCancelled else { return }
                let rejected = (error as? APIError).map { [400, 401, 403, 404, 409, 422].contains($0.status) } ?? false
                if rejected { self.pending = nil }
                self.error = "\(self.confirmed > 0 ? "Forwarded to \(self.confirmed) \(self.confirmed == 1 ? "destination" : "destinations"). " : "")\(rejected ? "Remaining forwards not sent." : "Remaining forwards not confirmed. Retry checks the same forwards.") \(error.localizedDescription)"
            }
        }
    }
    func stop() { task?.cancel(); task = nil }
}

/// Refreshes the forwarded source on sequence changes without losing the
/// oldest loaded reply, just as the reference ForwardConversationView does.
@MainActor @Observable final class NativeForwardConversation {
    let chat: ChatModel
    let messageID: String
    private(set) var conversation: ForwardConversationHistory?
    private(set) var loading = false
    private(set) var error: String?
    private var oldest: String?
    private var pages = 1
    @ObservationIgnored private var task: Task<Void, Never>?
    @ObservationIgnored private var sequence: String?
    @ObservationIgnored private let close: () -> Void

    init(chat: ChatModel, messageID: String, close: @escaping () -> Void) {
        self.chat = chat; self.messageID = messageID; self.close = close
    }
    var message: ChatMessage? { (chat.displayedMessages + chat.pinnedMessages).first { $0.id == messageID } }
    var available: Bool { message?.isDeleted == false && message?.forward?.message != nil }
    var root: ChatMessage? {
        guard available else { return nil }
        if let conversation { return conversation.root }
        return error == nil ? message?.forward?.message : nil
    }
    func refreshIfNeeded() {
        guard available else { stop(); close(); return }
        let next = message?.forward?.seq ?? "0"
        if sequence != next { sequence = next; load() }
    }
    func older() { guard !loading else { return }; pages += 1; load() }
    func load() {
        task?.cancel()
        guard available, let message else { conversation = nil; close(); return }
        loading = true; error = nil
        let pages = pages, oldest = oldest
        task = Task { [weak self, chat] in
            do {
                var fresh = try await chat.forwardedConversation(message: message)
                var loaded = 1
                while fresh.hasMore, let first = fresh.messages.first,
                      loaded < pages || oldest.map({ (try? Sequence.compare(first.seq, $0)) == .orderedDescending }) == true {
                    try Task.checkCancellation()
                    let earlier = try await chat.forwardedConversation(message: message, before: first.seq)
                    if earlier.messages.isEmpty { fresh.hasMore = false; break }
                    fresh.messages = earlier.messages + fresh.messages; fresh.hasMore = earlier.hasMore; loaded += 1
                }
                try Task.checkCancellation()
                self?.oldest = fresh.messages.first?.seq; self?.conversation = fresh
                self?.loading = false
            } catch is CancellationError { }
            catch {
                guard !Task.isCancelled else { return }
                self?.conversation = nil; self?.error = error.localizedDescription; self?.loading = false
            }
        }
    }
    func stop() { task?.cancel(); task = nil }
}
