import SwiftUI
import UniformTypeIdentifiers
import Observation

@MainActor @Observable final class NativeQuickDraft {
    let model: AppModel
    let person: MentionCardPerson
    let activity: ActivityItem?
    let draft: ProfileMessageDraft
    let close: () -> Void
    let autocomplete = ComposerAutocompleteController()
    var uploadsAvailable = false
    var showingFiles = false
    var showingEmoji = false
    var showingGIF = false
    var previewFile: URL?
    var editing: AttachmentEditRequest?
    var enabled = true
    @ObservationIgnored private var insertEmoji: ((String) -> Void)?
    private var active = true
    private let accountID: String?
    private let epoch: Int
    init(model: AppModel, person: MentionCardPerson, activity: ActivityItem? = nil, close: @escaping () -> Void) {
        self.model = model; self.person = person; self.activity = activity; self.close = close
        accountID = model.account?.id; epoch = model.accountEpoch
        draft = activity.map { model.activityDraft($0) } ?? model.profileMessageDraft(person: person)
    }
    var allowed: Bool {
        guard active, enabled, !draft.discarded, accountID != nil,
              model.account?.id == accountID, model.accountEpoch == epoch else { return false }
        if let activity { return !model.activityStale && model.canReplyToActivity(activity) }
        return !model.blockedIDs.contains(person.id)
    }
    var incoming: Bool { activity == nil && model.directMessages.contains { $0.peer.id == person.id && $0.status == .incoming } }
    var placeholder: String { activity?.replyLabel ?? "Message @\(person.username ?? person.title)" }
    private var recording: Bool { VoiceMessageRecorder.shared.isRecording(VoiceComposerID(profilePersonID: activity?.draftKey ?? person.id)) }
    var editable: Bool { allowed && draft.editable && !recording }
    var canSend: Bool { allowed && !incoming && draft.canSend && !draft.presentingDialog && !recording }
    var highlight: MentionSource {
        MentionSource(members: person.username.map { [.member(id: person.id, username: $0, displayName: person.title)] } ?? [], specials: false)
    }
    func start() async {
        async let gifs: () = model.chat.checkKlipyAvailability()
        do { _ = try await model.api.assetUsage(); try Task.checkCancellation(); uploadsAvailable = !draft.discarded && active }
        catch { uploadsAvailable = false }
        await gifs
    }
    func synchronizePresentation() { draft.presentingDialog = showingFiles || showingEmoji || showingGIF || previewFile != nil || editing != nil }
    func stop() { active = false; draft.presentingDialog = false; insertEmoji = nil }
    func send() {
        guard canSend else { return }
        Task { [self] in
            guard canSend else { return }
            if let activity { _ = await model.sendActivityReply(activity) }
            else if await model.sendProfileMessage(to: person) { close() }
        }
    }
    func sendGIF(_ gif: ChatGif, query: String) {
        guard allowed, !incoming else { return }
        Task { [self] in
            guard allowed, !incoming else { return }
            if let activity { _ = await model.sendActivityReply(activity, gif: gif, query: query) }
            else if await model.sendProfileGif(to: person, gif: gif, query: query) { close() }
        }
    }
    func openEmoji() { guard editable else { return }; insertEmoji = autocomplete.insertAction; showingEmoji = true; synchronizePresentation() }
    func pickEmoji(_ emoji: String) { if editable { insertEmoji?(emoji) }; showingEmoji = false; synchronizePresentation() }
    func finishPicker() { insertEmoji = nil; synchronizePresentation(); autocomplete.restoreFocus() }
    func pickFiles() { guard uploadsAvailable, editable else { return }; showingFiles = true; synchronizePresentation() }
    func paste() -> Bool {
        guard uploadsAvailable else { return false }; guard editable else { return true }
        do { let files = try NativeAttachmentPaste.stage(); guard !files.isEmpty else { return false }; draft.add(files) }
        catch { draft.error = "Pasted files couldn’t be added. Try selecting them with Files." }
        return true
    }
    func preview(_ file: ProfileMessageDraft.File) {
        guard editable else { return }
        if !file.editable { previewFile = file.editPreview ?? file.local.url; synchronizePresentation(); return }
        let id = file.id, draft = draft, kind = AttachmentKind.local(contentType: file.local.contentType)
        editing = AttachmentEditRequest(name: file.local.name, kind: kind, original: file.local.url, edit: file.edit,
            playback: kind == .video && file.editPreview == nil ? file.local.url : nil,
            spoiler: AttachmentDraftSpoiler(isOn: { draft.files.first { $0.id == id }?.spoiler ?? false }, toggle: { draft.toggleSpoiler(id: id) }),
            remove: { draft.remove(id: id) }) { edit, preview in draft.setEdit(id: id, edit: edit, preview: preview) }
        synchronizePresentation()
    }
    var voiceTarget: VoiceTarget {
        let draft = draft, model = model, person = person, activity = activity, close = close
        let ready = VoiceMessageSettings.shared.sendOnStop ? draft.canRecordVoice : draft.canStageVoice
        let reason = !uploadsAvailable ? "File uploads are unavailable"
            : draft.files.contains(where: { $0.voiceDurationMs != nil }) ? VoiceMessage.oneVoicePerMessage : "Send or retry the current message first."
        return VoiceTarget(owner: VoiceComposerID(profilePersonID: activity?.draftKey ?? person.id),
            enabled: uploadsAvailable && allowed && !incoming && ready && !draft.presentingDialog,
            unavailableReason: reason, identifier: "mention-card-voice",
            send: { recording in
                guard !draft.discarded else { AttachmentStaging.remove(recording.fileURL); return }
                if await model.sendProfileVoice(recording, to: person, activity: activity), activity == nil { close() }
            }, stage: { _ = draft.stageVoice($0) }, transcribe: { try await model.chat.dictate($0) },
            insert: { [weak self] in self?.editable == true && (self?.autocomplete.dictateAction?($0) ?? false) },
            isCurrent: { [weak self] in self?.allowed == true })
    }
    func retryVoice() {
        guard allowed, !incoming, draft.voice != nil, !draft.sending, !draft.presentingDialog else { return }
        Task { [self] in
            guard allowed, !incoming, !draft.sending, !draft.presentingDialog else { return }
            if await model.sendProfileVoice(nil, to: person, activity: activity), activity == nil { close() }
        }
    }
}

/// Compatibility entry point for the retired SwiftUI clients. The native
/// composer owns its presentations and does not host any SwiftUI descendants.
struct NativeQuickComposerSheet: View {
    @State private var state: NativeQuickDraft
    @Environment(\.isEnabled) private var enabled
    init(model: AppModel, person: MentionCardPerson, close: @escaping () -> Void) {
        _state = State(initialValue: NativeQuickDraft(model: model, person: person, close: close))
    }
    init(model: AppModel, activity: ActivityItem) {
        let person = MentionCardPerson(id: activity.actorId, username: activity.actorName, displayName: nil, avatarId: activity.avatarId, isSelf: false)
        _state = State(initialValue: NativeQuickDraft(model: model, person: person, activity: activity, close: {}))
    }
    var body: some View {
        NativeQuickComposerBridge(state: state)
            .accessibilityIdentifier("mention-card-composer-leaf")
            .onChange(of: enabled, initial: true) { _, value in state.enabled = value }
    }
}

private struct QuickComposerFiles: View {
    let state: NativeQuickDraft
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if state.incoming { Text("Open the conversation and accept this message request before replying.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted) }
            let failed = state.draft.files.filter { $0.uploadError != nil }.count
            if failed > 0 { Text(failed == 1 ? "1 file failed to upload. Send will retry it, or you can remove it." : "\(failed) files failed to upload. Send will retry them, or you can remove them.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.terracottaBright) }
            ForEach(state.draft.files.filter { $0.uploadError == AttachmentMetadata.refusalMessage }) { file in
                Text("\(file.spoiler ? "Spoiler attachment" : file.local.name): \(AttachmentMetadata.refusalMessage)").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                    .accessibilityIdentifier("attachment-metadata-refusal")
            }
            StagedFileLayout() {
                ForEach(state.draft.files) { file in
                    if let duration = file.voiceDurationMs {
                        VoiceDraftChip(id: file.id, url: file.local.url, durationMs: duration, waveform: file.voiceWaveform,
                            status: file.uploadError ?? (file.progress != nil ? "Uploading…" : nil), failed: file.uploadError != nil,
                            removable: state.editable, remove: { state.draft.remove(id: file.id) })
                    } else {
                        StagedFileChip(id: file.id, name: file.local.name, kind: AttachmentKind.local(contentType: file.local.contentType), localURL: file.local.url,
                            thumbnailURL: file.editPreview, status: file.uploadError ?? AttachmentPolicy.formatBytes(file.local.size), failed: file.uploadError != nil,
                            busyLabel: file.progress != nil ? "Uploading…" : nil, progress: file.progress, removable: state.editable, spoiler: file.spoiler,
                            toggleSpoiler: { state.draft.toggleSpoiler(id: file.id) }, previewable: file.editable && state.editable,
                            preview: { state.preview(file) }, remove: { state.draft.remove(id: file.id) })
                    }
                }
            }
        }.fixedSize(horizontal: false, vertical: true)
    }
}

private struct QuickComposerVoice: View {
    let state: NativeQuickDraft
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            VoiceComposerNotice(owner: state.voiceTarget.owner, identifier: "mention-card-voice")
            HStack(alignment: .bottom) {
                if let klipy = state.model.chat.klipy {
                    ComposerGifButton(configuration: klipy, enabled: state.editable && !state.incoming,
                        controller: state.autocomplete, accessibilityID: "mention-card-gif",
                        presenting: { state.showingGIF = $0; state.synchronizePresentation() }, send: state.sendGIF)
                }
                Spacer()
                Color.clear.frame(height: 42)
                    .modifier(VoiceComposerCover(target: state.voiceTarget, restoreFocus: { state.autocomplete.restoreFocus() }))
                VoiceRecordButton(target: state.voiceTarget)
            }
            #if os(iOS)
            .modifier(VoiceLockLayer())
            #endif
            if let voice = state.draft.voice, !state.draft.sending {
                Text(VoiceMessage.retryNotice(unconfirmed: voice.command != nil)).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                HStack {
                    Button("Retry", action: state.retryVoice).accessibilityIdentifier("mention-card-voice-retry")
                        .disabled(!state.allowed || state.draft.presentingDialog)
                    Button("Discard") { state.draft.discardVoice() }.accessibilityIdentifier("mention-card-voice-discard")
                }
            }
        }.preferredColorScheme(.dark).fixedSize(horizontal: false, vertical: true)
    }
}

#if os(macOS)
import AppKit

struct NativeQuickComposerBridge: NSViewRepresentable {
    @Bindable var state: NativeQuickDraft
    func makeNSView(context: Context) -> AppKitQuickComposer { AppKitQuickComposer(state: state) }
    // The view renders from its own observation. Rendering here, inside SwiftUI's
    // update, reads the editor's binding as its creation-time snapshot (the
    // empty draft) and resets the text being typed.
    func updateNSView(_ view: AppKitQuickComposer, context: Context) {}
    static func dismantleNSView(_ view: AppKitQuickComposer, coordinator: ()) { view.stop() }
    // Measuring here would lay out the hosted leaves inside this graph's
    // update, which aborts ("setting value during update"). The view measures
    // itself in AppKit's layout pass and invalidates its size when it changes.
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: AppKitQuickComposer, context: Context) -> CGSize? {
        guard let width = proposal.width else { return nil }
        return CGSize(width: width, height: nsView.measuredHeight)
    }
}

@MainActor final class AppKitQuickComposer: NSStackView, NSPopoverDelegate {
    let state: NativeQuickDraft
    private let observation = AppKitObservation()
    private var editor: NativeDialogComposer!
    private let files: AppKitStagedFiles
    private let session: NativeVoiceSession
    private let voice: AppKitVoiceControls
    private let recordingBar: AppKitVoiceControls
    private let voiceNotice: AppKitVoiceControls
    private let incoming = AppKitKit.label("Open the conversation and accept this message request before replying.", size: 12, color: AppKitKit.muted)
    private let voiceRetry = AppKitKit.stack([], spacing: 6)
    private let voiceRetryText = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var voiceRetryButton: AppKitKitButton!
    private let status = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var attach: AppKitKitButton!
    private var emoji: AppKitKitButton!
    private var gif: AppKitKitButton!
    private var send: AppKitKitButton!
    private var startTask: Task<Void, Never>?
    private var popover: NSPopover?
    private var sheet: NSWindow?
    private var filePanel: NSOpenPanel?
    private var presenting = false
    private var started = false
    private(set) var measuredHeight: CGFloat = 96
    init(state: NativeQuickDraft) {
        self.state = state; files = AppKitStagedFiles { NativeStagedFile.quick(state) }
        session = NativeVoiceSession { state.voiceTarget }
        voice = AppKitVoiceControls(session: session, kind: .microphone)
        recordingBar = AppKitVoiceControls(session: session, kind: .bar)
        voiceNotice = AppKitVoiceControls(session: session, kind: .notice)
        super.init(frame: .zero); orientation = .vertical; alignment = .leading; spacing = 8
        editor = NativeDialogComposer(text: Binding(get: { state.draft.text }, set: { if state.editable { state.draft.text = $0 } }),
            placeholder: state.placeholder, identifier: "mention-card-composer", height: 42, controller: state.autocomplete,
            submit: state.send, pasteAttachments: state.paste, highlightSource: state.highlight, grows: true)
        attach = AppKitKitButton("", kind: .plain) { state.pickFiles() }; attach.setAccessibilityIdentifier("mention-card-files")
        emoji = AppKitKitButton("", kind: .plain) { state.openEmoji() }; emoji.setAccessibilityIdentifier("mention-card-emoji")
        for (button, symbol, label) in [(attach!, "plus", "Attach files"), (emoji!, "face.smiling", "Insert emoji")] {
            button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil); button.imagePosition = .imageOnly
            button.setAccessibilityLabel(label); button.toolTip = label; button.widthAnchor.constraint(equalToConstant: 36).isActive = true
        }
        gif = AppKitKitButton("GIF", kind: .plain) { state.showingGIF = true; state.synchronizePresentation() }; gif.setAccessibilityIdentifier("mention-card-gif")
        send = AppKitKitButton("Send", kind: .primary) { state.send() }; send.setAccessibilityIdentifier("mention-card-send")
        let row = NSStackView(views: [attach, emoji, gif, NSView(), voice, send]); row.spacing = 4
        for child in [incoming, files, editor!, recordingBar, row, voiceNotice, voiceRetry, status] { addArrangedSubview(child); AppKitKit.fillWidth(child, of: self) }
        voiceRetryButton = AppKitKitButton("Retry") { state.retryVoice() }; voiceRetryButton.setAccessibilityIdentifier("mention-card-voice-retry")
        let discardVoice = AppKitKitButton("Discard") { state.draft.discardVoice() }; discardVoice.setAccessibilityIdentifier("mention-card-voice-discard")
        voiceRetry.addArrangedSubview(voiceRetryText); AppKitKit.fillWidth(voiceRetryText, of: voiceRetry)
        voiceRetry.addArrangedSubview(NSStackView(views: [voiceRetryButton, discardVoice]))
        files.heightChanged = { [weak self] in self?.needsLayout = true }
        status.setAccessibilityIdentifier("mention-card-error"); render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { startTask?.cancel(); session.stop(); state.stop() }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil {
            if !started { started = true; startTask = Task { [state] in await state.start() } }
            observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
        } else { observation.stop() }
    }
    func stop() { observation.stop(); startTask?.cancel(); session.stop(); closePresentation(); state.stop() }
    override func layout() {
        super.layout()
        guard bounds.width > 0 else { return }
        let width = widthAnchor.constraint(equalToConstant: bounds.width); width.isActive = true
        let height = ceil(fittingSize.height); width.isActive = false
        if abs(height - measuredHeight) > 0.5 {
            measuredHeight = height
            DispatchQueue.main.async { [weak self] in self?.invalidateIntrinsicContentSize() }
        }
    }
    override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: measuredHeight) }
    func render() {
        let recording = VoiceMessageRecorder.shared.isRecording(state.voiceTarget.owner)
        session.synchronize(); recordingBar.isHidden = !recording; editor.isHidden = recording
        incoming.isHidden = !state.incoming
        editor.update(editable: state.editable && !recording)
        attach.isEnabled = state.uploadsAvailable && state.editable && !state.draft.presentingDialog && state.draft.files.count < AttachmentPolicy.maxAttachments && !recording
        emoji.isEnabled = state.editable && !recording; send.isEnabled = state.canSend && !recording
        send.title = state.draft.pending == nil ? "Send" : "Retry send"
        status.stringValue = state.draft.error ?? (state.draft.pending != nil && !state.draft.sending ? "Delivery isn’t confirmed. Send retries the same message." : state.draft.progress ?? "")
        status.isHidden = status.stringValue.isEmpty
        files.isHidden = state.draft.files.isEmpty; files.render()
        gif.isHidden = state.model.chat.klipy == nil; gif.isEnabled = state.editable && !state.incoming && !state.draft.presentingDialog
        voiceRetryText.stringValue = VoiceMessage.retryNotice(unconfirmed: state.draft.voice?.command != nil)
        voiceRetryButton.isEnabled = state.allowed && !state.draft.presentingDialog
        voiceRetry.isHidden = state.draft.voice == nil || state.draft.sending
        presentIfNeeded()
    }
    private func presentIfNeeded() {
        guard !presenting, let window else { return }
        if state.showingFiles {
            presenting = true
            let panel = NSOpenPanel(); panel.canChooseFiles = true; panel.canChooseDirectories = false; panel.allowsMultipleSelection = true
            filePanel = panel
            panel.beginSheetModal(for: window) { [weak self] result in
                guard let self else { return }; self.filePanel = nil
                if result == .OK, self.state.allowed {
                    let files = panel.urls.prefix(AttachmentPolicy.maxAttachments).compactMap { try? AttachmentStaging.stage(copying: $0) }
                    self.state.draft.add(files)
                    if files.count < min(panel.urls.count, AttachmentPolicy.maxAttachments) { self.state.draft.error = "Some files couldn’t be added." }
                }
                self.closePresentation()
            }
        } else if state.showingEmoji || state.showingGIF {
            let popover = NSPopover(); popover.behavior = .transient; popover.delegate = self
            let close: () -> Void = { [weak self] in self?.closePresentation() }
            if state.showingEmoji {
                popover.contentViewController = AppKitReactionPicker(title: "Insert emoji", close: close) { [state] in state.pickEmoji($0); close() }
            } else if let configuration = state.model.chat.klipy {
                popover.contentViewController = AppKitKlipyPicker(configuration: configuration, close: close) { [state] gif, query in close(); state.sendGIF(gif, query: query) }
            } else { closePresentation(); return }
            presenting = true; self.popover = popover
            popover.show(relativeTo: (state.showingEmoji ? emoji : gif).bounds, of: state.showingEmoji ? emoji : gif, preferredEdge: .maxY)
        } else if let request = state.editing {
            presentSheet(AppKitAttachmentEditor(request: request) { [weak self] in self?.closePresentation() }, window: window)
        } else if let url = state.previewFile {
            presentSheet(AppKitFilePreview(url: url) { [weak self] in self?.closePresentation() }, window: window)
        }
    }
    private func presentSheet(_ controller: NSViewController, window: NSWindow) {
        let sheet = NSWindow(contentViewController: controller); sheet.setContentSize(NSSize(width: 720, height: 600))
        presenting = true; self.sheet = sheet; window.beginSheet(sheet)
    }
    func popoverDidClose(_ notification: Notification) { popover = nil; clearPresentation() }
    private func closePresentation() {
        let panel = filePanel; filePanel = nil; panel?.cancel(nil)
        let popover = popover; self.popover = nil; popover?.close()
        if let sheet { sheet.sheetParent?.endSheet(sheet); sheet.orderOut(nil); self.sheet = nil }
        clearPresentation()
    }
    private func clearPresentation() {
        presenting = false; state.showingFiles = false; state.showingEmoji = false; state.showingGIF = false
        state.previewFile = nil; state.editing = nil; state.finishPicker()
    }
}
#elseif os(iOS)
import UIKit

struct NativeQuickComposerBridge: UIViewControllerRepresentable {
    @Bindable var state: NativeQuickDraft
    func makeUIViewController(context: Context) -> UIKitQuickComposer { UIKitQuickComposer(state: state) }
    // The controller renders from its own observation. Rendering here, inside
    // SwiftUI's update, reads the editor's binding as its creation-time snapshot
    // (the empty draft) and resets the text being typed.
    func updateUIViewController(_ controller: UIKitQuickComposer, context: Context) {}
    static func dismantleUIViewController(_ controller: UIKitQuickComposer, coordinator: ()) { controller.stop() }
    func sizeThatFits(_ proposal: ProposedViewSize, uiViewController: UIKitQuickComposer, context: Context) -> CGSize? {
        guard let width = proposal.width else { return nil }
        return uiViewController.view.systemLayoutSizeFitting(CGSize(width: width, height: 0), withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel)
    }
}

@MainActor final class UIKitQuickComposer: UIViewController, UIAdaptivePresentationControllerDelegate {
    let state: NativeQuickDraft
    private let body = UIStackView()
    private var editor: NativeDialogComposer!
    private var files: UIKitStagedFiles!
    private var voice: UIKitVoiceControls!
    private var recordingBar: UIKitVoiceControls!
    private var voiceNotice: UIKitVoiceControls!
    private lazy var session = NativeVoiceSession { [state] in state.voiceTarget }
    private let incoming = UIKitMessageDialog.label("Open the conversation and accept this message request before replying.", size: 12)
    private let voiceRetry = UIStackView()
    private let voiceRetryText = UIKitMessageDialog.label("", size: 12, error: true)
    private var voiceRetryButton: UIButton!
    private let status = UIKitMessageDialog.label("", error: true)
    private var attach: UIButton!
    private var emoji: UIButton!
    private var gif: UIButton!
    private var send: UIButton!
    private var active = true
    private var startTask: Task<Void, Never>?
    private var presenting = false
    init(state: NativeQuickDraft) { self.state = state; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { startTask?.cancel(); state.stop() }
    override func loadView() {
        view = body; body.axis = .vertical; body.spacing = 8
        editor = NativeDialogComposer(text: Binding(get: { [state] in state.draft.text }, set: { [state] in if state.editable { state.draft.text = $0 } }),
            placeholder: state.placeholder, identifier: "mention-card-composer", height: 42, controller: state.autocomplete,
            submit: state.send, pasteAttachments: state.paste, highlightSource: state.highlight, grows: true)
        editor.heightChanged = { [weak self] in self?.view.invalidateIntrinsicContentSize() }
        files = UIKitStagedFiles { [state] in NativeStagedFile.quick(state) }
        voice = UIKitVoiceControls(session: session, kind: .microphone)
        recordingBar = UIKitVoiceControls(session: session, kind: .bar); voiceNotice = UIKitVoiceControls(session: session, kind: .notice)
        voiceRetry.axis = .vertical; voiceRetry.spacing = 6
        voiceRetryButton = UIKitMessageDialog.button("Retry") { [state] in state.retryVoice() }; voiceRetryButton.accessibilityIdentifier = "mention-card-voice-retry"
        let discardVoice = UIKitMessageDialog.button("Discard") { [state] in state.draft.discardVoice() }; discardVoice.accessibilityIdentifier = "mention-card-voice-discard"
        let retryActions = UIStackView(arrangedSubviews: [voiceRetryButton, discardVoice]); retryActions.spacing = 8
        voiceRetry.addArrangedSubview(voiceRetryText); voiceRetry.addArrangedSubview(retryActions)
        attach = UIKitMessageDialog.button("Attach files") { [state] in state.pickFiles() }; attach.accessibilityIdentifier = "mention-card-files"
        emoji = UIKitMessageDialog.button("Insert emoji") { [state] in state.openEmoji() }; emoji.accessibilityIdentifier = "mention-card-emoji"
        gif = UIKitMessageDialog.button("GIF") { [state] in state.showingGIF = true; state.synchronizePresentation() }; gif.accessibilityIdentifier = "mention-card-gif"
        send = UIKitMessageDialog.button("Send", primary: true) { [state] in state.send() }; send.accessibilityIdentifier = "mention-card-send"
        let row = UIStackView(arrangedSubviews: [attach, emoji, send]); row.spacing = 4; row.distribution = .fillEqually
        let voiceRow = UIStackView(arrangedSubviews: [gif, recordingBar, voice]); voiceRow.spacing = 8
        [incoming, files!, editor!, row, voiceRow, voiceNotice!, voiceRetry, status].forEach { body.addArrangedSubview($0) }
        files.heightChanged = { [weak self] in self?.view.invalidateIntrinsicContentSize() }
        status.accessibilityIdentifier = "mention-card-error"
    }
    override func viewDidLoad() { super.viewDidLoad(); startTask = Task { [state] in await state.start() }; AttachGalleryCache.warm(); observe() }
    func stop() { active = false; startTask?.cancel(); session.stop(); closePresentation(); state.stop() }
    private func observe() {
        guard active else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe() } }
    }
    func render() {
        guard isViewLoaded else { return }
        let recording = VoiceMessageRecorder.shared.isRecording(state.voiceTarget.owner)
        session.synchronize(); recordingBar.isHidden = !recording; editor.isHidden = recording; incoming.isHidden = !state.incoming
        editor.update(editable: state.editable && !recording)
        attach.isEnabled = state.uploadsAvailable && state.editable && !state.draft.presentingDialog && state.draft.files.count < AttachmentPolicy.maxAttachments && !recording
        emoji.isEnabled = state.editable && !recording; send.isEnabled = state.canSend && !recording
        send.configuration?.title = state.draft.pending == nil ? "Send" : "Retry send"
        status.text = state.draft.error ?? (state.draft.pending != nil && !state.draft.sending ? "Delivery isn’t confirmed. Send retries the same message." : state.draft.progress)
        status.isHidden = status.text == nil; files.isHidden = state.draft.files.isEmpty; files.render()
        gif.isHidden = state.model.chat.klipy == nil; gif.isEnabled = state.editable && !state.incoming && !state.draft.presentingDialog
        voiceRetryText.text = VoiceMessage.retryNotice(unconfirmed: state.draft.voice?.command != nil)
        voiceRetryButton.isEnabled = state.allowed && !state.draft.presentingDialog
        voiceRetry.isHidden = state.draft.voice == nil || state.draft.sending
        view.invalidateIntrinsicContentSize()
        presentIfNeeded()
    }
    private func presentIfNeeded() {
        guard !presenting, view.window != nil else { return }
        let close: () -> Void = { [weak self] in self?.closePresentation() }
        let controller: UIViewController
        if state.showingFiles {
            controller = UIKitAttachDrawer(mode: .attach(AttachDrawerDraft(files: { [state] in state.draft.files.map { AttachDrawerFile(id: $0.id, source: $0.local.source) } },
                add: { [state] in state.draft.add($0) }, remove: { [state] in state.draft.remove(id: $0) },
                canSend: { [state] in state.draft.canSend && !state.draft.sending }, send: { [state] in close(); state.send() },
                valid: { [state] in state.uploadsAvailable && state.editable })), close: close)
        } else if state.showingEmoji {
            controller = UIKitReactionPicker(title: "Insert emoji", close: close) { [state] in state.pickEmoji($0); close() }
        } else if state.showingGIF, let configuration = state.model.chat.klipy {
            controller = UIKitKlipyPicker(configuration: configuration, close: close) { [state] gif, query in close(); state.sendGIF(gif, query: query) }
        } else if let request = state.editing {
            controller = UIKitAttachmentEditor(request: request, close: close)
        } else if let url = state.previewFile { controller = UIKitFilePreview(url: url, close: close) }
        else { return }
        presenting = true
        if state.showingEmoji || state.showingGIF { UIKitHosting.sheet(controller, background: CaperTheme.surface) }
        controller.presentationController?.delegate = self
        present(controller, animated: !CaperAccessibility.reduceMotion)
    }
    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) { clearPresentation() }
    private func closePresentation() {
        presentedViewController?.dismiss(animated: false)
        clearPresentation()
    }
    private func clearPresentation() {
        presenting = false; state.showingFiles = false; state.showingEmoji = false; state.showingGIF = false
        state.previewFile = nil; state.editing = nil; state.finishPicker()
    }
}
#endif
