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
    /// Until the upload check answers, Attach looks ready instead of
    /// flashing from dimmed to enabled; a tap meanwhile opens once it answers.
    var uploadsChecked = false
    @ObservationIgnored private var wantsFiles = false
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
        uploadsChecked = true
        if wantsFiles { wantsFiles = false; pickFiles() }
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
    var canAttach: Bool { (uploadsAvailable || !uploadsChecked) && editable && draft.files.count < AttachmentPolicy.maxAttachments }
    func pickFiles() {
        guard editable, !draft.presentingDialog else { return }
        guard uploadsChecked else { wantsFiles = true; return }
        guard uploadsAvailable else { return }; showingFiles = true; synchronizePresentation()
    }
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

#if os(macOS)
import AppKit

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
            button.setAccessibilityLabel(label); button.toolTip = label; button.minimumHeight = 32
            button.widthAnchor.constraint(equalToConstant: 32).isActive = true
            button.heightAnchor.constraint(equalToConstant: 32).isActive = true
        }
        gif = AppKitKitButton("", kind: .plain) { state.showingGIF = true; state.synchronizePresentation() }; gif.setAccessibilityIdentifier("mention-card-gif")
        // The shared filled GIF badge, as wide as the emoji symbol beside it.
        if let badge = (NSImage(named: "caper-gif") ?? caperResourceBundle.image(forResource: "caper-gif"))?.copy() as? NSImage {
            badge.size = NSSize(width: 17, height: 17); badge.isTemplate = true
            gif.image = badge; gif.imagePosition = .imageOnly
        }
        gif.setAccessibilityLabel("GIF"); gif.toolTip = "GIF"; gif.minimumHeight = 32
        gif.widthAnchor.constraint(equalToConstant: 32).isActive = true
        gif.heightAnchor.constraint(equalToConstant: 32).isActive = true
        send = AppKitKitButton("Send", kind: .primary) { state.send() }; send.setAccessibilityIdentifier("mention-card-send")
        let row = NSStackView(views: [attach, emoji, gif, NSView(), voice, send]); row.spacing = 6
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
        attach.isEnabled = state.canAttach && !recording
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
    /// The draft or staged files changed height: a sheet sized to the
    /// composer measures again.
    var heightChanged: (() -> Void)?
    init(state: NativeQuickDraft) { self.state = state; super.init(nibName: nil, bundle: nil) }
    private func resized() { view.invalidateIntrinsicContentSize(); heightChanged?() }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { startTask?.cancel(); state.stop() }
    override func loadView() {
        view = body; body.axis = .vertical; body.spacing = 8
        editor = NativeDialogComposer(text: Binding(get: { [state] in state.draft.text }, set: { [state] in if state.editable { state.draft.text = $0 } }),
            placeholder: state.placeholder, identifier: "mention-card-composer", height: 42, controller: state.autocomplete,
            submit: state.send, pasteAttachments: state.paste, highlightSource: state.highlight, grows: true)
        editor.heightChanged = { [weak self] in self?.resized() }
        files = UIKitStagedFiles { [state] in NativeStagedFile.quick(state) }
        voice = UIKitVoiceControls(session: session, kind: .microphone)
        recordingBar = UIKitVoiceControls(session: session, kind: .bar); voiceNotice = UIKitVoiceControls(session: session, kind: .notice)
        voiceRetry.axis = .vertical; voiceRetry.spacing = 6
        voiceRetryButton = UIKitMessageDialog.button("Retry") { [state] in state.retryVoice() }; voiceRetryButton.accessibilityIdentifier = "mention-card-voice-retry"
        let discardVoice = UIKitMessageDialog.button("Discard") { [state] in state.draft.discardVoice() }; discardVoice.accessibilityIdentifier = "mention-card-voice-discard"
        let retryActions = UIStackView(arrangedSubviews: [voiceRetryButton, discardVoice]); retryActions.spacing = 8
        voiceRetry.addArrangedSubview(voiceRetryText); voiceRetry.addArrangedSubview(retryActions)
        attach = Self.tool(symbol: "plus", label: "Attach files") { [state] in state.pickFiles() }; attach.accessibilityIdentifier = "mention-card-files"
        attach.addAction(UIAction { _ in AttachGalleryCache.warm() }, for: .touchDown)
        emoji = Self.tool(symbol: "face.smiling", label: "Insert emoji") { [state] in state.openEmoji() }; emoji.accessibilityIdentifier = "mention-card-emoji"
        gif = Self.tool(image: UIKitVoiceControls.icon("gif", size: 22), label: "GIF") { [state] in state.showingGIF = true; state.synchronizePresentation() }; gif.accessibilityIdentifier = "mention-card-gif"
        send = UIKitMessageDialog.button("Send", primary: true) { [state] in state.send() }; send.accessibilityIdentifier = "mention-card-send"
        // One toolbar under the field, as the conversation composer: tools
        // on the left, the microphone and Send on the right.
        let spacer = UIView(); spacer.setContentHuggingPriority(.fittingSizeLevel, for: .horizontal)
        let row = UIStackView(arrangedSubviews: [attach, emoji, gif, spacer, voice, send]); row.spacing = 4; row.alignment = .center
        [attach!, emoji!, gif!, voice!, send!].forEach { $0.setContentHuggingPriority(.required, for: .horizontal) }
        row.setCustomSpacing(8, after: voice)
        [incoming, files!, editor!, recordingBar!, row, voiceNotice!, voiceRetry, status].forEach { body.addArrangedSubview($0) }
        files.heightChanged = { [weak self] in self?.resized() }
        status.accessibilityIdentifier = "mention-card-error"
    }
    override func viewDidLoad() { super.viewDidLoad(); startTask = Task { [state] in await state.start() }; observe() }
    /// A borderless toolbar control: a tap dims its glyph briefly and it
    /// never takes a filled or hover-like wash.
    private static func tool(symbol: String? = nil, image: UIImage? = nil, label: String, action: @escaping () -> Void) -> UIButton {
        var config = UIButton.Configuration.plain()
        config.image = symbol.flatMap { UIImage(systemName: $0, withConfiguration: UIImage.SymbolConfiguration(pointSize: 19, weight: .regular)) } ?? image
        config.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 8, bottom: 0, trailing: 8)
        let button = UIButton(configuration: config, primaryAction: UIAction { _ in action() })
        button.accessibilityLabel = label
        button.widthAnchor.constraint(greaterThanOrEqualToConstant: 40).isActive = true
        button.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        button.configurationUpdateHandler = { button in
            var config = button.configuration
            let color = UIColor(button.isEnabled ? CaperTheme.text : CaperTheme.muted)
            config?.baseForegroundColor = button.isHighlighted ? color.withAlphaComponent(0.5) : color
            config?.background.backgroundColor = .clear
            button.configuration = config
        }
        return button
    }
    func stop() { active = false; startTask?.cancel(); session.stop(); closePresentation(); state.stop() }
    private func observe() {
        guard active else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe() } }
    }
    func render() {
        guard isViewLoaded else { return }
        let recording = VoiceMessageRecorder.shared.isRecording(state.voiceTarget.owner)
        session.synchronize(); recordingBar.isHidden = !recording; editor.isHidden = recording; incoming.isHidden = !state.incoming
        attach.isHidden = recording; emoji.isHidden = recording
        editor.update(editable: state.editable && !recording)
        attach.isEnabled = state.canAttach && !recording
        emoji.isEnabled = state.editable && !recording; send.isEnabled = state.canSend && !recording
        send.configuration?.title = state.draft.pending == nil ? "Send" : "Retry send"
        status.text = state.draft.error ?? (state.draft.pending != nil && !state.draft.sending ? "Delivery isn’t confirmed. Send retries the same message." : state.draft.progress)
        status.isHidden = status.text == nil; files.isHidden = state.draft.files.isEmpty; files.render()
        gif.isHidden = recording || state.model.chat.klipy == nil; gif.isEnabled = state.editable && !state.incoming
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
