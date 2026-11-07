import SwiftUI

// Voice message UI (the shared voice-message contract): the composer's microphone,
// recording bar and staged voice chip, and the inline player with its transcript.

// MARK: Message

/// A `voice: true` attachment: an eggshell play button, the note's waveform
/// (played bars in the text colour, tap or drag to seek) and its time, with
/// the transcript beneath. Regular audio files keep `AttachmentAudioView`.
struct VoiceMessageView: View {
    let attachment: ChatAttachment
    let chat: ChatModel?
    @State private var waveformWidth: CGFloat = 0
    private var player: VoiceMessagePlayer { .shared }
    private static let width: CGFloat = 320

    var body: some View {
        let playing = player.isPlaying(attachment)
        let elapsed = player.elapsed(for: attachment)
        let label = VoiceDuration.playerLabel(durationMs: attachment.durationMs, elapsedMs: elapsed)
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                VoicePlayButton(playing: playing, size: 32) { player.toggle(attachment, chat: chat) }
                VoiceProgressWaveform(attachment: attachment, height: 28)
                    .frame(maxWidth: .infinity)
                    .contentShape(Rectangle())
                    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { waveformWidth = $0 }
                    #if os(iOS)
                    // Tap to seek; horizontal drags scrub while vertical ones keep scrolling the timeline.
                    .onTapGesture(coordinateSpace: .local) { location in seek(to: location.x) }
                    .simultaneousGesture(DragGesture(minimumDistance: 8).onChanged { value in
                        guard abs(value.translation.width) > abs(value.translation.height) else { return }
                        seek(to: value.location.x)
                    })
                    #else
                    .gesture(DragGesture(minimumDistance: 0).onChanged { value in seek(to: value.location.x) })
                    #endif
                    .accessibilityElement()
                    .accessibilityLabel("Voice message position")
                    .accessibilityValue("\(VoiceDuration.format(ms: elapsed ?? 0)) of \(VoiceDuration.format(ms: attachment.durationMs ?? 0))")
                    .accessibilityAdjustableAction { direction in
                        let progress = VoicePlaybackPosition.progress(elapsedMs: elapsed ?? 0, durationMs: attachment.durationMs)
                        player.seek(attachment, chat: chat, fraction: progress + (direction == .increment ? 0.1 : -0.1))
                    }
                Text(label).font(CaperTheme.font(12)).monospacedDigit().foregroundStyle(CaperTheme.muted)
                    .fixedSize()
                    .accessibilityLabel(elapsed == nil ? "Duration \(label)" : "\(label) remaining")
            }
            .padding(.leading, 8).padding(.trailing, 12).padding(.vertical, 6)
            .frame(maxWidth: Self.width, alignment: .leading)
            .background(CaperTheme.surface)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
            VoiceTranscriptView(transcript: attachment.transcript)
                .frame(maxWidth: Self.width, alignment: .leading)
        }
        .frame(maxWidth: Self.width, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("voice-message-\(attachment.id)")
    }

    /// Tap or drag across the waveform to move the playhead.
    private func seek(to x: CGFloat) {
        let fraction = VoicePlaybackPosition.fraction(x: Double(x), width: Double(waveformWidth))
        player.seek(attachment, chat: chat, fraction: fraction)
    }
}

/// The eggshell circular play/pause button of the player and the staged chip.
struct VoicePlayButton: View {
    let playing: Bool
    var size: CGFloat = 32
    let action: () -> Void

    var body: some View {
        Button(action: SwipeTapGuard.unlessSwiping(action)) {
            Image(systemName: playing ? "pause.fill" : "play.fill")
                .font(.system(size: size * 0.4, weight: .bold)).foregroundStyle(CaperTheme.onAction)
                .frame(width: size, height: size).background(Circle().fill(CaperTheme.action))
                #if os(iOS)
                .frame(minWidth: 44, minHeight: 44)
                #endif
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .modifier(ControlPointer())
        .accessibilityLabel(playing ? "Pause voice message" : "Play voice message")
    }
}

/// The note's waveform with its played portion, redrawn every display frame
/// while it plays so the playhead moves smoothly (the boundary bar fills
/// partly); otherwise drawn once.
struct VoiceProgressWaveform: View {
    let attachment: ChatAttachment
    var height: CGFloat = 28
    private var player: VoiceMessagePlayer { .shared }

    var body: some View {
        let playing = player.isPlaying(attachment)
        let elapsed = player.elapsed(for: attachment)
        TimelineView(.animation(paused: !playing)) { _ in
            let now = playing ? player.preciseElapsedMs(for: attachment) ?? elapsed : elapsed
            VoiceWaveformBars(source: .waveform(attachment.waveformBytes),
                              playedFraction: VoicePlaybackPosition.progress(elapsedMs: now ?? 0, durationMs: attachment.durationMs))
        }
        .frame(height: height)
    }
}

/// Evenly spaced 3-point bars with fully rounded caps and a 2-point gap,
/// vertically centred, at least 3 points tall so silence is a neat flat line.
/// Played bars (the boundary one partly) draw in the text colour. The live
/// recording meter is `VoiceLiveBars`.
struct VoiceWaveformBars: View {
    enum Source: Equatable {
        /// A sent note's waveform bytes (nil: flat line), linearly resampled to fit.
        case waveform([UInt8]?)
    }
    let source: Source
    var playedFraction: Double = 0
    var barWidth: CGFloat = 3
    var gap: CGFloat = 2
    var minimumHeight: CGFloat = 3
    var played = CaperTheme.text
    var unplayed = CaperTheme.muted.opacity(0.45)

    var body: some View {
        Canvas { context, size in
            let step = barWidth + gap
            let count = VoiceWaveform.barCount(available: Double(size.width), width: Double(barWidth), gap: Double(gap))
            switch source {
            case .waveform(let bytes):
                for (index, value) in VoiceWaveform.resample(bytes, count: count).enumerated() {
                    let rect = bar(x: CGFloat(index) * step, value: value, size: size)
                    let fill = VoicePlaybackPosition.playedFill(progress: playedFraction, bar: index, count: count)
                    draw(&context, rect, fill: CGFloat(fill))
                }
            }
        }
        .clipped()
        .accessibilityHidden(true)
    }

    private func bar(x: CGFloat, value: Double, size: CGSize) -> CGRect {
        let height = min(size.height, max(minimumHeight, CGFloat(value) * size.height))
        return CGRect(x: x, y: (size.height - height) / 2, width: barWidth, height: height)
    }

    private func draw(_ context: inout GraphicsContext, _ rect: CGRect, fill: CGFloat) {
        let shape = Path(roundedRect: rect, cornerRadius: barWidth / 2)
        if fill < 1 { context.fill(shape, with: .color(unplayed)) }
        guard fill > 0 else { return }
        var playedPart = context
        playedPart.clip(to: Path(CGRect(x: rect.minX, y: rect.minY, width: rect.width * fill, height: rect.height)))
        playedPart.fill(shape, with: .color(played))
    }
}

/// Slack-style transcript under the player: three lines then "View
/// transcript"; "Transcribing…" (breathing) while pending; nothing when
/// failed, absent, unknown or empty. The text is selectable.
struct VoiceTranscriptView: View {
    let transcript: VoiceTranscript?
    @State private var expanded = false
    @State private var fullHeight: CGFloat = 0
    @State private var collapsedHeight: CGFloat = 0

    var body: some View {
        switch transcript?.display ?? .hidden {
        case .hidden:
            EmptyView()
        case .pending:
            VoiceTranscriptPending()
        case .text(let text):
            VStack(alignment: .leading, spacing: 4) {
                Text(text)
                    .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                    .lineLimit(expanded ? nil : VoiceTranscript.clampedLines)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: {
                        if !expanded { collapsedHeight = $0 }
                    }
                    .background {
                        // The unclamped height, to know whether three lines hide anything.
                        Text(text).font(CaperTheme.font(13)).lineLimit(nil)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .hidden().accessibilityHidden(true)
                            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { fullHeight = $0 }
                    }
                    // Rebuild the selectable text when its line limit changes, so
                    // its drawing and the stack's measured height change together.
                    .id(expanded)
                    .transition(.identity)
                    .accessibilityIdentifier("voice-transcript-text")
                if expanded || fullHeight > collapsedHeight + 1 {
                    Button(expanded ? VoiceTranscript.collapseLabel : VoiceTranscript.expandLabel) {
                        // Text, toggle and timeline must relayout in one step,
                        // without crossfading the old and new text layouts.
                        var transaction = Transaction(animation: nil)
                        transaction.disablesAnimations = true
                        withTransaction(transaction) { expanded.toggle() }
                    }
                        .buttonStyle(.plain)
                        .font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.text)
                        .modifier(ControlPointer())
                        #if os(iOS)
                        .frame(minHeight: 32, alignment: .leading)
                        #endif
                        .accessibilityValue(expanded ? "Expanded" : "Collapsed")
                }
            }
            .modifier(VoiceTranscriptRule())
            // Keep the parent's identifier on the group, rather than replacing
            // the selectable text and toggle's identifiers on macOS 15.
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("voice-transcript")
        }
    }
}

/// "Transcribing…" breathes until the transcript replaces it: its opacity
/// eases in and out between 1 and 0.4, one cycle every 2 seconds. Reduce
/// Motion keeps it still. Only the label fades, not the rule beside it.
private struct VoiceTranscriptPending: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var dimmed = false

    var body: some View {
        Text(VoiceTranscript.pendingLabel)
            .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
            .opacity(dimmed && !reduceMotion ? VoiceTranscript.pendingBreathOpacity : 1)
            .modifier(VoiceTranscriptRule())
            .onAppear(perform: breathe)
            // A lazy timeline row that scrolls away drops its animation; start over when it returns.
            .onDisappear(perform: rest)
            .onChange(of: reduceMotion) { reduceMotion ? rest() : breathe() }
            .accessibilityIdentifier("voice-transcript-pending")
    }

    private func breathe() {
        guard !reduceMotion, !dimmed else { return }
        withAnimation(.easeInOut(duration: VoiceTranscript.pendingBreathSeconds / 2).repeatForever(autoreverses: true)) {
            dimmed = true
        }
    }

    private func rest() {
        var still = Transaction(animation: nil)
        still.disablesAnimations = true
        withTransaction(still) { dimmed = false }
    }
}

private struct VoiceTranscriptRule: ViewModifier {
    func body(content: Content) -> some View {
        content.padding(.leading, 10).padding(.vertical, 2)
            .overlay(alignment: .leading) { Rectangle().fill(CaperTheme.border).frame(width: 2) }
            .padding(.leading, 4)
    }
}

// MARK: Composer

/// Where a composer's voice message goes: the open conversation's channel or
/// thread composer, or a profile card's DM. A stopped recording is staged
/// in that composer by default (`stage`), or sent on its own with the
/// "Send voice messages as soon as I stop recording" setting (`send`).
/// Every microphone, bar and notice is built from one of these.
struct VoiceTarget {
    let owner: VoiceComposerID
    /// The microphone can start a recording now.
    let enabled: Bool
    /// Why it can't (the Mac's tooltip; VoiceOver's hint).
    let unavailableReason: String
    /// Prefix for the accessibility identifiers ("composer-voice-record", …).
    let identifier: String
    let send: @MainActor (VoiceRecording) async -> Void
    let stage: @MainActor (VoiceRecording) -> Void
    /// Dictation: the recording's words (`POST /api/dictation`), and putting
    /// them in the draft at its caret (false when the draft would be too long).
    /// Without both, a tap or click records a voice message.
    var transcribe: (@MainActor (VoiceRecording) async throws -> String)? = nil
    var insert: (@MainActor (String) -> Bool)? = nil
    /// Embedded native composers can disappear before the start task runs.
    var isCurrent: (@MainActor () -> Bool)? = nil

    var dictates: Bool { transcribe != nil && insert != nil }

    @MainActor static func chat(_ chat: ChatModel, inThread: Bool, controller: ComposerAutocompleteController? = nil) -> VoiceTarget {
        let sendsOnStop = VoiceMessageSettings.shared.sendOnStop
        let reason: String
        if !chat.canAttach { reason = chat.attachUnavailableReason }
        else if chat.attachmentFiles(inThread: inThread).contains(where: \.isVoice) { reason = VoiceMessage.oneVoicePerMessage }
        else if sendsOnStop { reason = "Send or dismiss the pending message first." }
        else { reason = "You can attach up to \(AttachmentPolicy.maxAttachments) files." }
        var transcribe: (@MainActor (VoiceRecording) async throws -> String)?
        var insert: (@MainActor (String) -> Bool)?
        if let controller {
            transcribe = { recording in try await chat.dictate(recording) }
            insert = { words in controller.dictateAction?(words) ?? false }
        }
        return VoiceTarget(owner: VoiceComposerID(chat: chat, inThread: inThread),
                           enabled: sendsOnStop ? chat.canRecordVoice(inThread: inThread) : chat.canStageVoice(inThread: inThread),
                           unavailableReason: reason,
                           identifier: inThread ? "thread-voice" : "composer-voice",
                           send: { await chat.sendVoice($0, inThread: inThread) },
                           stage: { chat.stageVoice($0, inThread: inThread) },
                           transcribe: transcribe, insert: insert)
    }

    /// Done or Stop (or a released hold, or the 10:00 limit): a dictation is
    /// transcribed into the draft; a voice message (or `asVoice`, the
    /// dictation bar's "Voice note") is staged or sent.
    @MainActor static func finish(_ target: VoiceTarget, asVoice: Bool = false) async {
        let recorder = VoiceMessageRecorder.shared
        let dictating = recorder.dictating && !asVoice && target.dictates
        guard recorder.isRecording(target.owner), let recording = await recorder.finish() else { return }
        if dictating { transcribe(recording, for: target) } else { await keep(recording, for: target) }
    }

    @MainActor static func keep(_ recording: VoiceRecording, for target: VoiceTarget) async {
        switch VoiceMessage.finish(sendsOnStop: VoiceMessageSettings.shared.sendOnStop) {
        case .send: await target.send(recording)
        case .stage: target.stage(recording)
        }
    }

    /// Transcribes and inserts; a failure keeps the audio for Retry or "Keep as voice note".
    @MainActor static func transcribe(_ recording: VoiceRecording, for target: VoiceTarget) {
        guard let transcribe = target.transcribe, let insert = target.insert else { return }
        let recorder = VoiceMessageRecorder.shared
        recorder.dictation?.task?.cancel()
        let fail = { (reason: String) in
            guard recorder.dictation?.recording == recording else { return }
            recorder.dictation = VoiceDictation(target: target, recording: recording, error: reason)
        }
        var dictation = VoiceDictation(target: target, recording: recording)
        dictation.task = Task { @MainActor in
            do {
                let words = try await transcribe(recording)
                guard !Task.isCancelled, recorder.dictation?.recording == recording else { return }
                if words.isEmpty { fail("No words heard.") }
                else if insert(words) {
                    recorder.dictation = nil
                    AttachmentStaging.remove(recording.fileURL)
                } else { fail("Messages must be 4,000 characters or fewer.") }
            } catch {
                guard !Task.isCancelled else { return }
                fail((error as? APIError)?.message ?? VoiceMessage.dictationError(status: nil))
            }
        }
        recorder.dictation = dictation
    }

    /// Cancel (while transcribing) or Discard (after a failure): the audio goes.
    @MainActor static func dropDictation() {
        let recorder = VoiceMessageRecorder.shared
        guard let dictation = recorder.dictation else { return }
        dictation.task?.cancel()
        recorder.dictation = nil
        AttachmentStaging.remove(dictation.recording.fileURL)
    }
}

/// The composer's microphone ("Dictate"), a tool button beside Emoji. A
/// click or tap dictates hands-free until Done: on the Mac the row becomes the
/// recording bar with Discard, "Voice note" and Done; on iPhone and iPad the
/// microphone itself becomes a pulsing terracotta Done and the keyboard
/// closes. Done transcribes the speech into the draft at its caret; "Voice
/// note" keeps the recording as a voice message instead. Touch layouts can
/// also press and hold for a voice message: release stops, and sliding up to
/// the lock keeps recording hands-free. VoiceOver activates it to start and
/// again to stop.
struct VoiceRecordButton: View {
    let target: VoiceTarget
    var height: CGFloat = 42
    private var recorder: VoiceMessageRecorder { .shared }
    private var owner: VoiceComposerID { target.owner }
    #if os(iOS)
    @State private var pressStart: Date?
    @State private var locked = false
    @GestureState private var pressing = false
    #endif

    init(target: VoiceTarget, height: CGFloat = 42) {
        self.target = target
        self.height = height
    }

    init(chat: ChatModel, inThread: Bool, height: CGFloat = 42, controller: ComposerAutocompleteController? = nil) {
        self.init(target: .chat(chat, inThread: inThread, controller: controller), height: height)
    }

    private var label: String { target.dictates ? VoiceMessage.dictateLabel : VoiceMessage.recordLabel }
    /// No new recording while this composer's last dictation is still transcribing or failed.
    private var enabled: Bool { target.enabled && recorder.dictation?.target.owner != owner }

    var body: some View {
        ZStack {
            #if os(iOS)
            if recorder.isActive(owner) && recorder.handsFree {
                VoiceStopButton(height: height, done: recorder.dictating) { stop() }
                    .accessibilityIdentifier("\(target.identifier)-stop")
            } else {
                holdableMicrophone
            }
            #else
            Button { begin(handsFree: true) } label: { microphone }
                .buttonStyle(.plain)
                .disabled(!enabled)
                .help(target.enabled ? label : target.unavailableReason)
                .accessibilityLabel(label)
                .accessibilityHint(target.enabled ? "" : target.unavailableReason)
                .accessibilityIdentifier("\(target.identifier)-record")
                .modifier(ControlPointer())
            #endif
        }
        .modifier(VoiceRecordingLifetime(owner: owner))
    }

    /// The same plain tool-button treatment as Attach and Emoji.
    private var microphone: some View {
        CaperIcon(name: "mic", size: 18)
            .foregroundStyle(CaperTheme.muted)
            .frame(width: 36, height: height)
            .contentShape(Rectangle())
    }

    private func begin(handsFree: Bool) {
        guard enabled else { return }
        let target = target
        Task {
            guard target.isCurrent?() != false else { return }
            await VoiceMessageRecorder.shared.start(for: target.owner, handsFree: handsFree, dictating: target.dictates) {
                // 10:00: stop, then stage or send, on every platform.
                Task { @MainActor in await VoiceTarget.finish(target) }
            }
        }
    }

    private func stop() {
        let target = target
        Task { await VoiceTarget.finish(target) }
    }

    /// A released hold: always a voice message.
    private func stopVoice() {
        let target = target
        Task { await VoiceTarget.finish(target, asVoice: true) }
    }

    #if os(iOS)
    private var holdableMicrophone: some View {
        let enabled = enabled
        let active = recorder.isActive(owner)
        return CaperIcon(name: "mic", size: 18)
            .foregroundStyle(active ? CaperTheme.text : CaperTheme.muted)
            .frame(width: 30, height: 30)
            // Held: the microphone under the finger is live terracotta.
            .background(RoundedRectangle(cornerRadius: 8).fill(active ? CaperTheme.terracotta : .clear))
            .frame(width: 36, height: height)
            .contentShape(Rectangle())
            // The composer box clips its contents, so `VoiceLockLayer` outside it draws the lock.
            .anchorPreference(key: VoiceLockAnchorKey.self, value: .bounds) { active && recorder.holding ? $0 : nil }
            .opacity(enabled || active ? 1 : 0.45)
            .highPriorityGesture(holdGesture, including: enabled || active ? .all : .subviews)
            .onChange(of: pressing) { _, now in
                guard !now else { return }
                // The system took the touch (such as the permission alert): keep
                // recording hands-free rather than losing it; Stop or Discard ends it.
                Task { @MainActor in
                    await Task.yield()
                    guard pressStart != nil else { return }
                    pressStart = nil; locked = false
                    if recorder.isActive(owner) { goHandsFree() }
                }
            }
            .accessibilityElement()
            .accessibilityLabel(label)
            .accessibilityHint(enabled ? (target.dictates ? "Dictates until you’re done. Hold to record a voice message instead." : "Records until you stop. You can also hold to record and release to stop.") : target.unavailableReason)
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { begin(handsFree: true) }
            .accessibilityIdentifier("\(target.identifier)-record")
            .disabled(!enabled && !active)
    }

    private var holdGesture: some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .local)
            .updating($pressing) { _, state, _ in state = true }
            .onChanged { value in
                if pressStart == nil {
                    let start = Date()
                    pressStart = start; locked = false
                    begin(handsFree: false)
                    // Still down after a tap's length: a voice message. Show what
                    // releasing and sliding do.
                    Task { @MainActor in
                        try? await Task.sleep(for: .seconds(VoiceMessage.tapThreshold))
                        if pressStart == start, !locked, recorder.isActive(owner), !recorder.handsFree {
                            VoiceMessageRecorder.haptic()
                            recorder.holding = true; recorder.dictating = false
                        }
                    }
                }
                guard !locked, recorder.isActive(owner), !recorder.handsFree else { return }
                let lift = min(0, Double(value.translation.height))
                recorder.lift = lift
                if VoiceMessage.locksBySliding(translation: lift) {
                    locked = true
                    VoiceMessageRecorder.haptic()
                    recorder.lift = 0; recorder.holding = false; recorder.handsFree = true; recorder.dictating = false
                }
            }
            .onEnded { _ in
                guard let start = pressStart else { return }
                pressStart = nil
                let wasLocked = locked
                locked = false
                recorder.lift = 0
                // A start that failed (denied permission) already shows its notice.
                guard recorder.isActive(owner) else { return }
                let recorded = recorder.isRecording(owner) ? recorder.elapsedMs : 0
                switch VoiceMessage.release(pressDuration: Date().timeIntervalSince(start), recordedMs: recorded, locked: wasLocked) {
                case .discard: recorder.cancel()
                case .handsFree: goHandsFree()
                case .stop: stopVoice()
                }
            }
    }

    /// The finger has lifted and the recording carries on until Stop: the
    /// keyboard closes, so typing can't reach the field hidden under the bar.
    private func goHandsFree() {
        recorder.handsFree = true; recorder.holding = false; recorder.lift = 0
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    }
    #endif
}

/// The Stop of a hands-free recording ("Stop recording"): a terracotta
/// square that pulses once a second while the microphone is live (still
/// under Reduce Motion).
struct VoiceStopButton: View {
    var height: CGFloat = 42
    var width: CGFloat = 36
    var square: CGFloat = 30
    /// Dictating: a check ("Done, insert text") instead of the square.
    var done = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: done ? "checkmark" : "stop.fill").font(.system(size: done ? 14 : 12, weight: .bold))
                .frame(width: square, height: square)
                .frame(width: width, height: height)
                .contentShape(Rectangle())
        }
        .buttonStyle(VoiceStopStyle(square: square))
        .help(done ? VoiceMessage.doneLabel : VoiceMessage.stopLabel)
        .accessibilityLabel(done ? VoiceMessage.doneLabel : VoiceMessage.stopLabel)
    }
}

/// Terracotta fill with a text-coloured glyph on the glyph's square only,
/// and a ring that grows and fades from it every second.
private struct VoiceStopStyle: ButtonStyle {
    let square: CGFloat
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(CaperTheme.text)
            .background {
                ZStack {
                    TimelineView(.animation(paused: reduceMotion)) { context in
                        let phase = reduceMotion ? 1 : context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1)
                        let grow = 1 - pow(1 - phase, 3)
                        RoundedRectangle(cornerRadius: 8 + 7 * grow)
                            .stroke(CaperTheme.terracottaBright.opacity(0.55 * (1 - grow)), lineWidth: 2)
                            .frame(width: square + 14 * grow, height: square + 14 * grow)
                    }
                    RoundedRectangle(cornerRadius: 8)
                        .fill(configuration.isPressed ? CaperTheme.terracotta : CaperTheme.terracottaBright)
                        .frame(width: square, height: square)
                }
            }
            .modifier(ControlPointer())
    }
}

/// Where a held microphone is, for `VoiceLockLayer`.
struct VoiceLockAnchorKey: PreferenceKey {
    static let defaultValue: Anchor<CGRect>? = nil
    static func reduce(value: inout Anchor<CGRect>?, nextValue: () -> Anchor<CGRect>?) { value = value ?? nextValue() }
}

/// Draws `VoiceLockHint` above a held microphone inside this view. Applied
/// outside the composer box, whose rounded clip would cut it off.
struct VoiceLockLayer: ViewModifier {
    private var recorder: VoiceMessageRecorder { .shared }

    func body(content: Content) -> some View {
        content.overlayPreferenceValue(VoiceLockAnchorKey.self) { anchor in
            GeometryReader { proxy in
                if let anchor {
                    let mic = proxy[anchor]
                    VoiceLockHint(progress: VoiceMessage.lockProgress(translation: recorder.lift))
                        .position(x: mic.midX, y: mic.minY - 8 - VoiceLockHint.height / 2)
                        .modifier(Entrance.anchored)
                }
            }
        }
    }
}

/// Above a held microphone: a lock with an arrow under it. Sliding up lifts
/// the lock toward the top and fades the arrow; reaching it locks the
/// recording hands-free.
struct VoiceLockHint: View {
    let progress: Double
    /// Two 14-point icons 8 apart with 10 points above and below.
    static let height: CGFloat = 56

    var body: some View {
        VStack(spacing: 8) {
            CaperIcon(name: "lock", size: 14).foregroundStyle(CaperTheme.text)
                .offset(y: -6 * progress)
            CaperIcon(name: "chevron-up", size: 14).foregroundStyle(CaperTheme.muted)
                .opacity(1 - progress)
        }
        .padding(.vertical, 10)
        .frame(width: 32, height: Self.height)
        .background(RoundedRectangle(cornerRadius: 8).fill(CaperTheme.raised))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .fixedSize()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// A recording ends with its composer: closing it, or opening another
/// conversation or thread in it, discards the recording.
private struct VoiceRecordingLifetime: ViewModifier {
    let owner: VoiceComposerID
    func body(content: Content) -> some View {
        content
            .onChange(of: owner) { old, _ in
                if VoiceMessageRecorder.shared.isActive(old) { VoiceMessageRecorder.shared.cancel() }
            }
            .onDisappear {
                if VoiceMessageRecorder.shared.isActive(owner) { VoiceMessageRecorder.shared.cancel() }
            }
    }
}

/// While this composer records, its row (on the Mac the whole row with
/// Send; on iPhone and iPad the field beside the microphone) is covered by
/// the recording bar. On the Mac it is also disabled, so typing and Return
/// can't reach the message field.
struct VoiceComposerCover: ViewModifier {
    let target: VoiceTarget
    var height: CGFloat = 42
    /// Returns the keyboard to the message field once the bar closes (macOS).
    var restoreFocus: (() -> Void)? = nil
    private var recorder: VoiceMessageRecorder { .shared }

    init(target: VoiceTarget, height: CGFloat = 42, restoreFocus: (() -> Void)? = nil) {
        self.target = target; self.height = height; self.restoreFocus = restoreFocus
    }

    init(chat: ChatModel, inThread: Bool, height: CGFloat = 42, controller: ComposerAutocompleteController? = nil,
         restoreFocus: (() -> Void)? = nil) {
        self.init(target: .chat(chat, inThread: inThread, controller: controller), height: height, restoreFocus: restoreFocus)
    }

    func body(content: Content) -> some View {
        let recording = recorder.isRecording(target.owner)
        content
            .opacity(recording ? 0 : 1)
            .allowsHitTesting(!recording)
            #if os(macOS)
            // Gives up keyboard focus, so Return reaches the bar's Stop rather than the message field.
            .disabled(recording)
            #endif
            // iPhone keeps the keyboard up while a finger holds the microphone, so
            // the composer doesn't move under it; it closes once recording goes hands-free.
            .accessibilityHidden(recording)
            .overlay(alignment: .bottom) {
                if recording {
                    VoiceRecordingBar(target: target, height: height)
                        .modifier(Entrance.anchored)
                }
            }
            .onChange(of: recording) { _, now in
                if !now { restoreFocus?() }
            }
    }
}

/// The recording bar: `[trash]  ● 0:07  live bars  [Voice note] [Done]` on
/// the Mac while dictating, `[Stop]` for a voice message (Return stops,
/// Escape discards). On touch devices it shows Discard, the timer, live bars
/// and, while dictating, "Voice note", with the microphone beside it turned
/// into Done (or Stop) once the finger lifts; while held, Discard steps out
/// (releasing stops).
struct VoiceRecordingBar: View {
    let target: VoiceTarget
    var height: CGFloat = 42
    private var recorder: VoiceMessageRecorder { .shared }

    var body: some View {
        #if os(iOS)
        HStack(spacing: 10) {
            discard.opacity(recorder.holding ? 0 : 1).disabled(recorder.holding)
            VoiceRecordingTimer(elapsedMs: recorder.elapsedMs)
            VoiceLiveBars()
            if recorder.dictating { asVoiceNote }
        }
        .padding(.trailing, recorder.dictating ? 4 : 12)
        .frame(maxWidth: .infinity, minHeight: height, maxHeight: height)
        .background(CaperTheme.composer)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(recorder.dictating ? "Dictating" : "Recording voice message")
        .accessibilityIdentifier("\(target.identifier)-recording")
        #else
        HStack(spacing: 10) {
            discard.keyboardShortcut(.cancelAction)
            VoiceRecordingTimer(elapsedMs: recorder.elapsedMs)
            VoiceLiveBars()
            if recorder.dictating { asVoiceNote }
            VoiceStopButton(height: height, width: 42, square: 32, done: recorder.dictating) {
                let target = target
                Task { await VoiceTarget.finish(target) }
            }
            .keyboardShortcut(.defaultAction)
            .help("\(recorder.dictating ? VoiceMessage.doneLabel : VoiceMessage.stopLabel) (Return)")
            .accessibilityIdentifier("\(target.identifier)-stop")
        }
        .padding(.leading, 4)
        .frame(maxWidth: .infinity, minHeight: height, maxHeight: height)
        .background(CaperTheme.composer)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(recorder.dictating ? "Dictating" : "Recording voice message")
        .accessibilityIdentifier("\(target.identifier)-recording")
        #endif
    }

    /// Dictating: stop now and keep the recording as a voice message.
    private var asVoiceNote: some View {
        Button {
            let target = target
            Task { await VoiceTarget.finish(target, asVoice: true) }
        } label: {
            HStack(spacing: 6) {
                CaperIcon(name: "audio-lines", size: 14)
                Text(VoiceMessage.asVoiceNoteLabel).font(CaperTheme.font(12, weight: .bold))
            }
            .foregroundStyle(CaperTheme.text)
            .padding(.horizontal, 10)
            .frame(height: 32)
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .fixedSize()
        .help(VoiceMessageSettings.shared.sendOnStop ? "Stop and send as a voice message" : "Stop and keep as a voice message")
        .accessibilityIdentifier("\(target.identifier)-as-note")
        .modifier(ControlPointer())
    }

    private var discard: some View {
        Button { recorder.cancel() } label: {
            CaperIcon(name: "trash-2", size: 16).foregroundStyle(CaperTheme.muted)
                .frame(width: 36, height: 36).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(recorder.dictating ? VoiceMessage.discardDictationLabel : VoiceMessage.discardLabel)
        .accessibilityLabel(recorder.dictating ? VoiceMessage.discardDictationLabel : VoiceMessage.discardLabel)
        .accessibilityIdentifier("\(target.identifier)-discard")
        .modifier(ControlPointer())
    }

}

/// The recording dot (steady: Stop carries the pulse) and m:ss.
struct VoiceRecordingTimer: View {
    let elapsedMs: Int

    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(CaperTheme.terracottaBright).frame(width: 8, height: 8)
            Text(VoiceDuration.format(ms: elapsedMs))
                .font(CaperTheme.font(13, weight: .medium)).monospacedDigit().foregroundStyle(CaperTheme.text)
        }
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Recording, \(VoiceDuration.format(ms: elapsedMs))")
    }
}

/// The live input meter (`VoiceLiveMeter`'s pinch bars), redrawn every
/// frame so it scrolls right to left at a constant rate with the forming bar
/// at the right edge (new bars step in without the slide under Reduce Motion).
struct VoiceLiveBars: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var recorder: VoiceMessageRecorder { .shared }

    var body: some View {
        let levels = recorder.levels
        TimelineView(.animation(paused: reduceMotion)) { context in
            let scroll = reduceMotion ? 0 : VoiceWaveform.scrollFraction(sinceLastBar: context.date.timeIntervalSince(recorder.lastBarAt))
            let pending = recorder.pendingLevel
            Canvas { graphics, size in
                for bar in VoiceLiveMeter.layout(levels, pending: pending, width: Double(size.width), height: Double(size.height), scroll: scroll) {
                    let rect = CGRect(x: bar.x, y: bar.y, width: bar.width, height: bar.height)
                    let color = VoiceLiveMeter.color(brightness: bar.brightness)
                    graphics.fill(Path(roundedRect: rect, cornerRadius: min(rect.width, rect.height) / 2),
                                  with: .color(Color(red: color.red, green: color.green, blue: color.blue, opacity: bar.alpha)))
                }
            }
        }
        .frame(height: 22)
        .frame(maxWidth: .infinity)
        .clipped()
        .accessibilityHidden(true)
    }
}

/// The microphone's inline message for one composer (denied permission, a
/// failed recording), and on touch devices the release hint above a held
/// recording.
struct VoiceComposerNotice: View {
    let owner: VoiceComposerID
    let identifier: String
    private var recorder: VoiceMessageRecorder { .shared }

    init(owner: VoiceComposerID, identifier: String) {
        self.owner = owner; self.identifier = identifier
    }

    init(chat: ChatModel, inThread: Bool) {
        self.init(owner: VoiceComposerID(chat: chat, inThread: inThread), identifier: inThread ? "thread-voice" : "composer-voice")
    }

    var body: some View {
        #if os(iOS)
        if recorder.isRecording(owner) && recorder.holding {
            Text(VoiceMessage.releaseHint(sendsOnStop: VoiceMessageSettings.shared.sendOnStop))
                .font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.text)
                .frame(maxWidth: .infinity)
                .accessibilityHidden(true)
        }
        #endif
        if let text = recorder.notice(for: owner) {
            Text(text)
                .font(CaperTheme.font(11))
                .foregroundStyle(CaperTheme.error)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("\(identifier)-notice")
        }
        if let dictation = recorder.dictation, dictation.target.owner == owner {
            if let error = dictation.error {
                VStack(alignment: .leading, spacing: 6) {
                    Text(VoiceMessage.dictationFailed(error))
                        .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 8) {
                        action("Retry", id: "retry") { VoiceTarget.transcribe(dictation.recording, for: dictation.target) }
                        if dictation.target.enabled {
                            action(VoiceMessageSettings.shared.sendOnStop ? "Send as voice note" : "Keep as voice note", id: "keep") {
                                recorder.dictation = nil
                                Task { await VoiceTarget.keep(dictation.recording, for: dictation.target) }
                            }
                        }
                        action("Discard", id: "discard-dictation") { VoiceTarget.dropDictation() }
                    }
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("\(identifier)-dictation-failed")
            } else {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(VoiceMessage.transcribing).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                    action("Cancel", id: "cancel-dictation") { VoiceTarget.dropDictation() }
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("\(identifier)-transcribing")
            }
        }
    }

    private func action(_ title: String, id: String, perform: @escaping () -> Void) -> some View {
        Button(action: perform) {
            Text(title).font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.text)
                .padding(.horizontal, 8).frame(height: 26)
                .background(RoundedRectangle(cornerRadius: 8).fill(CaperTheme.raised))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("\(identifier)-\(id)")
        .modifier(ControlPointer())
    }
}

/// A staged voice note among the composer's files: play/pause, a small
/// waveform, its duration (or upload state) and Remove. Sent with the
/// message's text and files on Send.
struct VoiceDraftChip: View {
    let id: String
    let url: URL
    let durationMs: Int
    let waveform: [UInt8]?
    /// Upload state or error; nil once uploaded.
    var status: String?
    var failed = false
    var retry: (() -> Void)?
    var removable = true
    let remove: () -> Void
    private var player: VoiceMessagePlayer { .shared }

    /// What the shared player plays: the local recording.
    private var attachment: ChatAttachment {
        ChatAttachment(id: "draft-\(id)", kind: .audio, contentType: VoiceMessage.uploadContentType, name: VoiceMessage.label, size: 0,
                       durationMs: durationMs, url: url.absoluteString, voice: true, waveform: waveform.map(VoiceWaveform.encode))
    }

    var body: some View {
        let note = attachment
        let playing = player.isPlaying(note)
        ZStack(alignment: .topTrailing) {
            VStack(alignment: .leading, spacing: 6) {
                VoicePlayButton(playing: playing, size: 28) { player.toggle(note, chat: nil) }
                    .frame(width: 28, height: 28)
                VoiceProgressWaveform(attachment: note, height: 18)
                Text(status ?? VoiceDuration.playerLabel(durationMs: durationMs, elapsedMs: player.elapsed(for: note)))
                    .font(CaperTheme.font(11, weight: .medium)).monospacedDigit()
                    .foregroundStyle(failed ? CaperTheme.terracottaBright : CaperTheme.muted)
                    .lineLimit(1)
            }
            .padding(8)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(CaperTheme.surface)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(failed ? CaperTheme.terracotta : CaperTheme.border))
            HStack(spacing: 2) {
                if let retry {
                    Button(action: retry) {
                        CaperIcon(name: "rotate-ccw", size: 12).frame(width: 24, height: 24).background(Circle().fill(CaperTheme.raised))
                    }
                    .buttonStyle(.plain).help("Retry upload").accessibilityLabel("Retry voice message upload")
                }
                if removable {
                    Button(action: remove) {
                        CaperIcon(name: "x", size: 12).frame(width: 24, height: 24).background(Circle().fill(CaperTheme.raised))
                    }
                    .buttonStyle(.plain).help("Remove voice message").accessibilityLabel("Remove voice message")
                    .accessibilityIdentifier("voice-draft-remove")
                }
            }
            .foregroundStyle(CaperTheme.text)
            .padding(4)
        }
        .onDisappear { if player.isCurrent(note) { player.stop() } }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Voice message, \(VoiceDuration.format(ms: durationMs))\(status.map { ", \($0)" } ?? "")")
        .accessibilityIdentifier("voice-draft")
    }
}

/// The Mac's Settings section for "Send voice messages as soon as I stop
/// recording" (iPhone has the same toggle in the account menu's Audio settings).
struct VoiceMessageSettingsSection: View {
    @Bindable var settings: VoiceMessageSettings

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Voice messages").font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
            Toggle(isOn: $settings.sendOnStop) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(VoiceMessage.sendOnStopTitle).font(CaperTheme.font(14))
                    Text(VoiceMessage.sendOnStopDetail).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                }
            }
            .toggleStyle(.switch)
            .accessibilityIdentifier("voice-messages-send-on-stop")
        }
    }
}
