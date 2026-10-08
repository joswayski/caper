import SwiftUI

struct MessageEditorView: View {
    @Bindable var chat: ChatModel
    @State private var baseline: ChatMessage
    @State private var draft: String
    @State private var busy = false
    @State private var error: String?
    @FocusState private var focused: Bool
    let close: () -> Void

    init(chat: ChatModel, message: ChatMessage, close: @escaping () -> Void) {
        self.chat = chat; self.close = close
        _baseline = State(initialValue: message); _draft = State(initialValue: message.content.text)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("Edit message").font(CaperTheme.font(20, weight: .bold))
                Text("Previous versions remain visible to people who can read this message.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                TextEditor(text: $draft).focused($focused).frame(minHeight: 150, maxHeight: 280)
                    .font(CaperTheme.font(14)).scrollContentBackground(.hidden).padding(8)
                    .background(CaperTheme.composer, in: RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border)).disabled(busy)
                    .accessibilityLabel("Message").accessibilityIdentifier("message-edit-text")
                Text("\(draft.unicodeScalars.count) / 4,000").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                if let error {
                    Text(error).foregroundStyle(CaperTheme.terracottaBright)
                    Button("Discard draft and load latest") { perform {
                        baseline = try await chat.reloadMessage(baseline); draft = baseline.content.text; focused = true
                    } }.disabled(busy)
                }
                HStack {
                    Spacer()
                    Button("Cancel", action: close).disabled(busy).keyboardShortcut(.cancelAction)
                    Button(busy ? "Saving…" : "Save changes") { perform { try await chat.editMessage(baseline, text: draft); close() } }
                        .buttonStyle(.borderedProminent).tint(CaperTheme.terracotta)
                        .modifier(ControlPointer())
                        .disabled(busy || MessageValidation.error(for: draft) != nil || !chat.canEdit(baseline))
                        .keyboardShortcut(.return, modifiers: .command)
                }
            }.padding(20)
        }.background(CaperTheme.raised)
            #if os(macOS)
            .frame(width: 520, height: 430)
            #endif
            .interactiveDismissDisabled(busy)
            .onAppear { focused = true }
            .onChange(of: chat.editingContext) { _, _ in close() }
    }

    private func perform(_ operation: @escaping @MainActor () async throws -> Void) {
        busy = true; error = nil
        Task { @MainActor in
            do { try await operation() }
            catch is CancellationError { return }
            catch { self.error = error.localizedDescription; focused = true }
            busy = false
        }
    }
}

struct MessageHistoryView: View {
    @Bindable var chat: ChatModel
    let message: ChatMessage
    let close: () -> Void
    @State private var versions: [MessageVersion] = []
    @State private var selected = 0
    @State private var loading = false
    @State private var more = false
    @State private var error: String?
    @State private var request = 0
    private var currentRevision: Int { (chat.messages + chat.pinnedMessages).first { $0.id == message.id }?.revision ?? message.revision ?? 1 }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack { Text("Message history").font(CaperTheme.font(20, weight: .bold)); Spacer(); Button("Close", action: close).keyboardShortcut(.cancelAction) }
                Text("\(message.author.name) · Previous versions are retained.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                if versions.count > 1 { MessageVersionComparison(before: versions[1], after: versions[0], current: true) }
                else if let original = versions.first { Text("Original version"); Text(original.content.text).textSelection(.enabled) }
                if versions.count > 1 {
                    Divider()
                    Picker("View previous versions", selection: $selected) {
                        Text("Choose an earlier version…").tag(0)
                        ForEach(versions.dropFirst()) { version in Text(versionLabel(version)).tag(version.revision) }
                    }.pickerStyle(.menu)
                    if let version = versions.first(where: { $0.revision == selected }) {
                        if let before = versions.first(where: { $0.revision == selected - 1 }) { MessageVersionComparison(before: before, after: version) }
                        else {
                            Text(version.revision == 1 ? "Original version" : "Version \(version.revision)")
                            Text(version.content.text).font(.system(size: 12, design: .monospaced)).textSelection(.enabled)
                            if version.revision > 1 && more { Text("Load older versions to compare this change.").foregroundStyle(CaperTheme.muted) }
                        }
                    }
                }
                if loading { ProgressView("Loading versions…") }
                if let error { Text(error).foregroundStyle(CaperTheme.terracottaBright); Button("Retry") { Task { await load(older: !versions.isEmpty) } }.disabled(loading) }
                if more && error == nil { Button("Load older versions") { Task { await load(older: true) } }.disabled(loading) }
            }.padding(20)
        }.background(CaperTheme.raised)
            #if os(macOS)
            .frame(width: 820, height: 580)
            #endif
            .task(id: currentRevision) { await load(older: false) }
            .onChange(of: chat.editingContext) { _, _ in close() }
    }

    private func load(older: Bool) async {
        request += 1
        let currentRequest = request
        loading = true; error = nil
        do {
            let page = try await chat.messageVersions(message, before: older ? versions.last?.revision : nil)
            guard !Task.isCancelled, currentRequest == request else { return }
            versions = older ? versions + page.versions.filter { item in !versions.contains { $0.revision == item.revision } } : page.versions
            more = page.hasMore
            if !older { selected = 0 }
        } catch is CancellationError { return }
        catch { if currentRequest == request { self.error = error.localizedDescription } }
        if currentRequest == request { loading = false }
    }
}

private func versionLabel(_ version: MessageVersion) -> String {
    let date = ChatDateDivider.date(version.createdAt)?.formatted(date: .abbreviated, time: .shortened) ?? version.createdAt
    return "\(version.revision == 1 ? "Original version" : "Version \(version.revision)") · \(date)"
}

private struct MessageVersionComparison: View {
    let before: MessageVersion
    let after: MessageVersion
    var current = false
    var body: some View {
        let diff = messageDiff(before: before.content.text, after: after.content.text)
        HStack(alignment: .top, spacing: 12) {
            column(before, title: current ? "Previous version" : "Before", tokens: diff.0, color: CaperTheme.terracotta)
            column(after, title: current ? "Current version" : "After", tokens: diff.1, color: Color(red: 99/255, green: 122/255, blue: 67/255))
        }
    }
    private func column(_ version: MessageVersion, title: String, tokens: [MessageDiffToken], color: Color) -> some View {
        var text = AttributedString()
        for token in tokens {
            var span = AttributedString(token.text)
            if token.changed { span.backgroundColor = color.opacity(0.3) }
            text.append(span)
        }
        return VStack(alignment: .leading, spacing: 8) {
            Text(title).font(CaperTheme.font(14, weight: .bold))
            Text(versionLabel(version)).font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
            Text(text).font(.system(size: 12, design: .monospaced)).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
        }.frame(maxWidth: .infinity, alignment: .topLeading)
    }
}
