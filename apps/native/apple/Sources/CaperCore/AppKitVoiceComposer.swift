#if os(macOS)
import AppKit
import Observation
import SwiftUI

/// The shared recording controls surround, but never replace, the native editor.
/// A target captures this composer's lifetime before permission, audio decoding
/// or transcription can suspend; a late result cannot move into another draft.
@MainActor @Observable public final class AppKitVoiceComposer {
    private let chat: ChatModel
    private let inThread: Bool
    @ObservationIgnored private weak var editor: NSTextView?
    private var owner: VoiceComposerID
    private var context: String
    private var generation = 0
    private var visible = true
    @ObservationIgnored private lazy var microphone: NSHostingView<AppKitVoiceControl> = NSHostingView(rootView: AppKitVoiceControl(bar: false) { [weak self] in self?.target })
    @ObservationIgnored private lazy var recordingBar: AppKitVoiceBar = AppKitVoiceBar(rootView: AppKitVoiceControl(bar: true) { [weak self] in self?.target })

    public init(chat: ChatModel, inThread: Bool, editor: NSTextView) {
        self.chat = chat; self.inThread = inThread; self.editor = editor
        owner = VoiceComposerID(chat: chat, inThread: inThread)
        context = chat.editingContext
    }

    public var button: NSView { microphone }
    public var bar: NSView { recordingBar }
    public var isRecording: Bool { VoiceMessageRecorder.shared.isRecording(VoiceComposerID(chat: chat, inThread: inThread)) }
    public var hasNotice: Bool {
        let current = VoiceComposerID(chat: chat, inThread: inThread)
        return VoiceMessageRecorder.shared.notice(for: current) != nil || VoiceMessageRecorder.shared.dictation?.target.owner == current
    }

    /// Native hiding does not reliably unmount a SwiftUI leaf. End its lifetime
    /// explicitly on folding, access loss, account/conversation/root changes.
    public func synchronize(visible: Bool) {
        let next = VoiceComposerID(chat: chat, inThread: inThread)
        guard owner != next || context != chat.editingContext || self.visible != visible else { return }
        if owner != next || context != chat.editingContext || (self.visible && !visible) { cancel() }
        owner = next; context = chat.editingContext; self.visible = visible
    }

    public func cancel() {
        generation += 1
        let recorder = VoiceMessageRecorder.shared
        if recorder.isActive(owner) { recorder.cancel() }
        if recorder.dictation?.target.owner == owner { VoiceTarget.dropDictation() }
        recorder.clearNotice(for: owner)
    }

    private var target: VoiceTarget {
        let base = VoiceTarget.chat(chat, inThread: inThread)
        let context = chat.editingContext, generation = self.generation
        let current = { [weak self] in
            guard let self else { return false }
            return self.visible && self.editor?.window != nil && self.chat.canInteract
                && self.generation == generation && self.chat.editingContext == context
                && VoiceComposerID(chat: self.chat, inThread: self.inThread) == base.owner
        }
        let keep = { (recording: VoiceRecording) -> Bool in
            guard current() else {
                if VoiceMessageRecorder.shared.dictation?.recording == recording { VoiceTarget.dropDictation() }
                else { AttachmentStaging.remove(recording.fileURL) }
                return false
            }
            return true
        }
        return VoiceTarget(owner: base.owner, enabled: visible && base.enabled, unavailableReason: base.unavailableReason,
            identifier: base.identifier,
            send: { recording in if keep(recording) { await base.send(recording) } },
            stage: { recording in if keep(recording) { base.stage(recording) } },
            transcribe: { [weak self] recording in
                guard keep(recording), let self else { throw CancellationError() }
                let words = try await self.chat.dictate(recording)
                guard keep(recording) else { throw CancellationError() }
                return words
            },
            insert: { [weak self] words in current() && (self?.insert(words) ?? false) },
            isCurrent: current)
    }

    /// Uses the same spacing, scalar limit and UTF-16 repair as the shipping
    /// composer; insertText retains native selection, delegate updates and undo.
    @discardableResult public func insert(_ words: String) -> Bool {
        guard visible, context == chat.editingContext, owner == VoiceComposerID(chat: chat, inThread: inThread),
              let editor, editor.isEditable, !editor.hasMarkedText(), chat.canInteract else { return false }
        let selection = editor.selectedRange()
        let words = VoiceMessage.dictationText(in: editor.string, selection: selection, words: words)
        guard !words.isEmpty, let result = EmojiAutocomplete.inserting(words, in: editor.string, selection: selection) else { return false }
        editor.breakUndoCoalescing()
        editor.insertText(result.text, replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        editor.setSelectedRange(result.selection)
        editor.breakUndoCoalescing()
        return true
    }
}

@MainActor private struct AppKitVoiceControl: View {
    let bar: Bool
    let target: () -> VoiceTarget?

    var body: some View {
        Group {
            if let target = target() {
                let recording = VoiceMessageRecorder.shared.isRecording(target.owner)
                if bar {
                    if recording {
                        GeometryReader { geometry in
                            VoiceRecordingBar(target: target, compact: geometry.size.width < 300)
                        }.modifier(Entrance.anchored)
                    }
                } else {
                    // Keep the microphone's recording-lifetime modifier mounted
                    // while the bar covers it, as VoiceComposerCover does.
                    VoiceRecordButton(target: target)
                        .opacity(recording ? 0 : 1).allowsHitTesting(!recording)
                        .accessibilityHidden(recording)
                }
            }
        }.preferredColorScheme(.dark).tint(CaperTheme.terracottaBright)
    }
}

/// The native text view must give up Return/Escape while its bar covers it.
/// Handle those keys even when focus is on the hosting view, not a SwiftUI button.
private final class AppKitVoiceBar: NSHostingView<AppKitVoiceControl> {
    override var acceptsFirstResponder: Bool { true }

    override func keyDown(with event: NSEvent) {
        if event.modifierFlags.intersection([.command, .option, .control, .shift]).isEmpty,
           let target = rootView.target(), VoiceMessageRecorder.shared.isRecording(target.owner) {
            if event.keyCode == 53 { VoiceMessageRecorder.shared.cancel(); return }
            if event.keyCode == 36 || event.keyCode == 76 {
                Task { await VoiceTarget.finish(target) }
                return
            }
        }
        super.keyDown(with: event)
    }
}
#endif
