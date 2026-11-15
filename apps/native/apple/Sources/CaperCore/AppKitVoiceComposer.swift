#if os(macOS)
import AppKit
import Observation

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
    @ObservationIgnored private lazy var session = NativeVoiceSession { [weak self, chat, inThread] in
        self?.target ?? VoiceTarget(owner: VoiceComposerID(chat: chat, inThread: inThread), enabled: false,
            unavailableReason: "Composer is closed", identifier: inThread ? "thread-voice" : "composer-voice",
            send: { AttachmentStaging.remove($0.fileURL) }, stage: { AttachmentStaging.remove($0.fileURL) }, isCurrent: { false })
    }
    @ObservationIgnored private lazy var microphone = AppKitVoiceControls(session: session, kind: .microphone)
    @ObservationIgnored private lazy var recordingBar = AppKitVoiceControls(session: session, kind: .bar)

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
        session.stop()
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
    /// composer, as one undoable edit that notifies the delegate.
    @discardableResult public func insert(_ words: String) -> Bool {
        guard visible, context == chat.editingContext, owner == VoiceComposerID(chat: chat, inThread: inThread),
              let editor, editor.isEditable, !editor.hasMarkedText(), chat.canInteract else { return false }
        let selection = editor.selectedRange()
        let words = VoiceMessage.dictationText(in: editor.string, selection: selection, words: words)
        guard !words.isEmpty, let result = EmojiAutocomplete.inserting(words, in: editor.string, selection: selection) else { return false }
        editor.breakUndoCoalescing()
        Self.replace(editor, with: result.text, selection: result.selection)
        editor.breakUndoCoalescing()
        return true
    }

    /// NSTextView's own undo of a replacement restores the characters without
    /// textDidChange, leaving the dictated words in the draft. Register the step
    /// here instead, so undo and redo are ordinary edits the delegate sees.
    private static func replace(_ editor: NSTextView, with text: String, selection: NSRange) {
        let previous = editor.string, previousSelection = editor.selectedRange()
        let all = NSRange(location: 0, length: previous.utf16.count)
        let undo = editor.undoManager
        undo?.disableUndoRegistration()
        if editor.shouldChangeText(in: all, replacementString: text) {
            editor.textStorage?.replaceCharacters(in: all, with: NSAttributedString(string: text, attributes: editor.typingAttributes))
            editor.setSelectedRange(selection)
            editor.didChangeText()
        }
        undo?.enableUndoRegistration()
        guard editor.string != previous else { return }
        undo?.registerUndo(withTarget: editor) { editor in
            MainActor.assumeIsolated { replace(editor, with: previous, selection: previousSelection) }
        }
    }
}

#endif
