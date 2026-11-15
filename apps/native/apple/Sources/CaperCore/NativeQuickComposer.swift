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

/// The retained recorder, GIF chooser and attachment chips are separate leaves;
/// the rich field, formatting, autocomplete and composer actions are native.
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
        #if os(macOS)
        ZStack {
            content.opacity(state.editing == nil ? 1 : 0).disabled(state.editing != nil).accessibilityHidden(state.editing != nil)
            if let request = state.editing {
                NativeAttachmentEditorBridge(request: request) { state.editing = nil; state.synchronizePresentation() }
                    .frame(width: 720, height: 600).id(request.id)
            }
        }.preference(key: ProfileComposerPresentationSize.self, value: state.editing == nil ? nil : CGSize(width: 720, height: 600))
        #else
        content.fullScreenCover(item: $state.editing, onDismiss: state.synchronizePresentation) { request in
            NativeAttachmentEditorBridge(request: request) { state.editing = nil; state.synchronizePresentation() }
                .preferredColorScheme(.dark)
        }
        #endif
    }
    private var content: some View {
        NativeQuickComposerBridge(state: state)
            .accessibilityIdentifier("mention-card-composer-leaf")
            .task { await state.start() }
            .onChange(of: enabled, initial: true) { _, value in state.enabled = value }
            .onChange(of: state.showingFiles) { _, _ in state.synchronizePresentation() }
            .onChange(of: state.showingEmoji) { _, _ in state.synchronizePresentation() }
            .onChange(of: state.showingGIF) { _, _ in state.synchronizePresentation() }
            .onChange(of: state.previewFile) { _, _ in state.synchronizePresentation() }
            .quickLookPreview($state.previewFile)
            #if os(macOS)
            .popover(isPresented: $state.showingEmoji) { ReactionPicker(title: "Insert emoji", select: state.pickEmoji).onDisappear(perform: state.finishPicker) }
            .fileImporter(isPresented: $state.showingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
                defer { state.synchronizePresentation() }
                guard case .success(let urls) = result else { return }
                let files = urls.prefix(AttachmentPolicy.maxAttachments).compactMap { try? AttachmentStaging.stage(copying: $0) }
                state.draft.add(files)
                if files.count < min(urls.count, AttachmentPolicy.maxAttachments) { state.draft.error = "Some files couldn’t be added." }
            }
            #else
            .sheet(isPresented: $state.showingEmoji, onDismiss: state.finishPicker) { ReactionPicker(title: "Insert emoji", select: state.pickEmoji) }
            .fullScreenCover(isPresented: $state.showingFiles, onDismiss: state.synchronizePresentation) {
                AttachDrawer(draft: AttachDrawerDraft(files: { state.draft.files.map { AttachDrawerFile(id: $0.id, source: $0.local.source) } },
                    add: { state.draft.add($0) }, remove: { state.draft.remove(id: $0) }, canSend: { state.draft.canSend && !state.draft.sending },
                    send: { state.showingFiles = false; state.synchronizePresentation(); state.send() }, valid: { state.uploadsAvailable && state.editable }))
            }.task { AttachGalleryCache.warm() }
            #endif
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
    func updateNSView(_ view: AppKitQuickComposer, context: Context) { view.render() }
    static func dismantleNSView(_ view: AppKitQuickComposer, coordinator: ()) { view.stop() }
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: AppKitQuickComposer, context: Context) -> CGSize? {
        guard let width = proposal.width else { return nil }
        nsView.setFrameSize(CGSize(width: width, height: nsView.frame.height)); nsView.layoutSubtreeIfNeeded()
        return CGSize(width: width, height: nsView.fittingSize.height)
    }
}

@MainActor final class AppKitQuickComposer: NSStackView {
    private let state: NativeQuickDraft
    private let observation = AppKitObservation()
    private var editor: NativeDialogComposer!
    private let files: NSHostingView<AnyView>
    private let voice: NSHostingView<AnyView>
    private let status = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var attach: AppKitKitButton!
    private var emoji: AppKitKitButton!
    private var send: AppKitKitButton!
    private var contentWidth: CGFloat = 0
    init(state: NativeQuickDraft) {
        self.state = state; files = NSHostingView(rootView: AnyView(QuickComposerFiles(state: state)))
        voice = NSHostingView(rootView: AnyView(QuickComposerVoice(state: state)))
        super.init(frame: .zero); orientation = .vertical; alignment = .leading; spacing = 8
        editor = NativeDialogComposer(text: Binding(get: { state.draft.text }, set: { if state.editable { state.draft.text = $0 } }),
            placeholder: state.placeholder, identifier: "mention-card-composer", height: 42, controller: state.autocomplete,
            submit: state.send, pasteAttachments: state.paste, highlightSource: state.highlight, grows: true)
        attach = AppKitKitButton("Attach files", kind: .plain) { state.pickFiles() }; attach.setAccessibilityIdentifier("mention-card-files")
        emoji = AppKitKitButton("Insert emoji", kind: .plain) { state.openEmoji() }; emoji.setAccessibilityIdentifier("mention-card-emoji")
        send = AppKitKitButton("Send", kind: .primary) { state.send() }; send.setAccessibilityIdentifier("mention-card-send")
        let row = NSStackView(views: [attach, emoji, NSView(), send]); row.spacing = 4
        files.sizingOptions = [.intrinsicContentSize]; voice.sizingOptions = [.intrinsicContentSize]
        for child in [files, editor!, row, voice, status] { addArrangedSubview(child); AppKitKit.fillWidth(child, of: self) }
        status.setAccessibilityIdentifier("mention-card-error"); render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) } else { observation.stop() }
    }
    func stop() { observation.stop(); state.stop() }
    override func layout() {
        if bounds.width > 0, contentWidth != bounds.width {
            contentWidth = bounds.width
            files.rootView = AnyView(QuickComposerFiles(state: state).frame(width: contentWidth))
            voice.rootView = AnyView(QuickComposerVoice(state: state).frame(width: contentWidth))
        }
        super.layout()
    }
    func render() {
        let recording = VoiceMessageRecorder.shared.isRecording(state.voiceTarget.owner)
        editor.update(editable: state.editable && !recording)
        attach.isEnabled = state.uploadsAvailable && state.editable && !state.draft.presentingDialog && state.draft.files.count < AttachmentPolicy.maxAttachments && !recording
        emoji.isEnabled = state.editable && !recording; send.isEnabled = state.canSend && !recording
        send.title = state.draft.pending == nil ? "Send" : "Retry send"
        status.stringValue = state.draft.error ?? (state.draft.pending != nil && !state.draft.sending ? "Delivery isn’t confirmed. Send retries the same message." : state.draft.progress ?? "")
        status.isHidden = status.stringValue.isEmpty
        files.isHidden = state.draft.files.isEmpty && !state.incoming
    }
}
#elseif os(iOS)
import UIKit

struct NativeQuickComposerBridge: UIViewControllerRepresentable {
    @Bindable var state: NativeQuickDraft
    func makeUIViewController(context: Context) -> UIKitQuickComposer { UIKitQuickComposer(state: state) }
    func updateUIViewController(_ controller: UIKitQuickComposer, context: Context) { controller.render() }
    static func dismantleUIViewController(_ controller: UIKitQuickComposer, coordinator: ()) { controller.stop() }
    func sizeThatFits(_ proposal: ProposedViewSize, uiViewController: UIKitQuickComposer, context: Context) -> CGSize? {
        guard let width = proposal.width else { return nil }
        return uiViewController.view.systemLayoutSizeFitting(CGSize(width: width, height: 0), withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel)
    }
}

@MainActor final class UIKitQuickComposer: UIViewController {
    private let state: NativeQuickDraft
    private let body = UIStackView()
    private var editor: NativeDialogComposer!
    private var files: UIHostingController<AnyView>!
    private var voice: UIHostingController<AnyView>!
    private let status = UIKitMessageDialog.label("", error: true)
    private var attach: UIButton!
    private var emoji: UIButton!
    private var send: UIButton!
    private var active = true
    init(state: NativeQuickDraft) { self.state = state; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() {
        view = body; body.axis = .vertical; body.spacing = 8
        editor = NativeDialogComposer(text: Binding(get: { [state] in state.draft.text }, set: { [state] in if state.editable { state.draft.text = $0 } }),
            placeholder: state.placeholder, identifier: "mention-card-composer", height: 42, controller: state.autocomplete,
            submit: state.send, pasteAttachments: state.paste, highlightSource: state.highlight, grows: true)
        files = UIHostingController(rootView: AnyView(QuickComposerFiles(state: state))); voice = UIHostingController(rootView: AnyView(QuickComposerVoice(state: state)))
        for host in [files!, voice!] { host.sizingOptions = [.intrinsicContentSize]; host.view.backgroundColor = .clear; addChild(host) }
        attach = UIKitMessageDialog.button("Attach files") { [state] in state.pickFiles() }; attach.accessibilityIdentifier = "mention-card-files"
        emoji = UIKitMessageDialog.button("Insert emoji") { [state] in state.openEmoji() }; emoji.accessibilityIdentifier = "mention-card-emoji"
        send = UIKitMessageDialog.button("Send", primary: true) { [state] in state.send() }; send.accessibilityIdentifier = "mention-card-send"
        let row = UIStackView(arrangedSubviews: [attach, emoji, send]); row.spacing = 4; row.distribution = .fillEqually
        [files.view!, editor!, row, voice.view!, status].forEach { body.addArrangedSubview($0) }
        for host in [files!, voice!] { host.didMove(toParent: self) }
        status.accessibilityIdentifier = "mention-card-error"
    }
    override func viewDidLoad() { super.viewDidLoad(); observe() }
    func stop() { active = false; state.stop() }
    private func observe() {
        guard active else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe() } }
    }
    func render() {
        guard isViewLoaded else { return }
        let recording = VoiceMessageRecorder.shared.isRecording(state.voiceTarget.owner)
        editor.update(editable: state.editable && !recording)
        attach.isEnabled = state.uploadsAvailable && state.editable && !state.draft.presentingDialog && state.draft.files.count < AttachmentPolicy.maxAttachments && !recording
        emoji.isEnabled = state.editable && !recording; send.isEnabled = state.canSend && !recording
        send.configuration?.title = state.draft.pending == nil ? "Send" : "Retry send"
        status.text = state.draft.error ?? (state.draft.pending != nil && !state.draft.sending ? "Delivery isn’t confirmed. Send retries the same message." : state.draft.progress)
        status.isHidden = status.text == nil; files.view.isHidden = state.draft.files.isEmpty && !state.incoming
        view.invalidateIntrinsicContentSize()
    }
}
#endif
