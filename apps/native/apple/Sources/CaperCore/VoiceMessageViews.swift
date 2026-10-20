import SwiftUI

// Voice message UI (the shared voice-message contract): the composer's microphone and
// recording bar, and the inline player with its transcript.

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
        let progress = VoicePlaybackPosition.progress(elapsedMs: elapsed ?? 0, durationMs: attachment.durationMs)
        let label = VoiceDuration.playerLabel(durationMs: attachment.durationMs, elapsedMs: elapsed)
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Button(action: SwipeTapGuard.unlessSwiping { player.toggle(attachment, chat: chat) }) {
                    Image(systemName: playing ? "pause.fill" : "play.fill")
                        .font(.system(size: 13, weight: .bold)).foregroundStyle(CaperTheme.onAction)
                        .frame(width: 32, height: 32).background(Circle().fill(CaperTheme.action))
                        #if os(iOS)
                        .frame(minWidth: 44, minHeight: 44)
                        #endif
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .modifier(ControlPointer())
                .accessibilityLabel(playing ? "Pause voice message" : "Play voice message")
                VoiceWaveformBars(source: .waveform(attachment.waveformBytes), playedFraction: progress)
                    .frame(height: 28)
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
                        let step = direction == .increment ? 0.1 : -0.1
                        player.seek(attachment, chat: chat, fraction: progress + step)
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

/// Rounded vertical bars; the first `playedFraction` draw in the text colour.
struct VoiceWaveformBars: View {
    enum Source: Equatable {
        /// A sent note's waveform bytes (nil: flat placeholder), resampled to fit.
        case waveform([UInt8]?)
        /// Live recording levels, newest at the right.
        case live([Float])

        func heights(count: Int) -> [Double] {
            switch self {
            case .waveform(let bytes): return VoiceWaveform.bars(bytes, count: count)
            case .live(let levels): return VoiceWaveform.liveBars(levels, count: count)
            }
        }
    }
    let source: Source
    var playedFraction: Double = 0
    var barWidth: CGFloat = 3
    var gap: CGFloat = 2
    var played = CaperTheme.text
    var unplayed = CaperTheme.muted.opacity(0.45)

    var body: some View {
        Canvas { context, size in
            let count = VoiceWaveform.barCount(available: Double(size.width), width: Double(barWidth), gap: Double(gap))
            let heights = source.heights(count: count)
            let playedCount = VoicePlaybackPosition.playedBars(progress: playedFraction, count: count)
            for (index, value) in heights.enumerated() {
                let height = max(3, CGFloat(value) * size.height)
                let rect = CGRect(x: CGFloat(index) * (barWidth + gap), y: (size.height - height) / 2, width: barWidth, height: height)
                context.fill(Path(roundedRect: rect, cornerRadius: barWidth / 2),
                             with: .color(index < playedCount ? played : unplayed))
            }
        }
        .accessibilityHidden(true)
    }
}

/// Slack-style transcript under the player: three lines then "View
/// transcript"; "Transcribing…" while pending; nothing when failed, absent,
/// unknown or empty. The text is selectable.
struct VoiceTranscriptView: View {
    let transcript: VoiceTranscript?
    @State private var expanded = false
    @State private var fullHeight: CGFloat = 0
    @State private var shownHeight: CGFloat = 0

    var body: some View {
        switch transcript?.display ?? .hidden {
        case .hidden:
            EmptyView()
        case .pending:
            Text(VoiceTranscript.pendingLabel)
                .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                .modifier(VoiceTranscriptRule())
                .accessibilityIdentifier("voice-transcript-pending")
        case .text(let text):
            VStack(alignment: .leading, spacing: 4) {
                Text(text)
                    .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                    .lineLimit(expanded ? nil : VoiceTranscript.clampedLines)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { shownHeight = $0 }
                    .background {
                        // The unclamped height, to know whether three lines hide anything.
                        Text(text).font(CaperTheme.font(13)).lineLimit(nil)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .hidden().accessibilityHidden(true)
                            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { fullHeight = $0 }
                    }
                if expanded || fullHeight > shownHeight + 1 {
                    Button(expanded ? VoiceTranscript.collapseLabel : VoiceTranscript.expandLabel) { expanded.toggle() }
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
            .accessibilityIdentifier("voice-transcript")
        }
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

/// The composer's microphone ("Record voice message"), next to Emoji. On the
/// Mac a click replaces the composer row with the recording bar. On iPhone
/// and iPad press and hold to record, release to send, slide left to cancel;
/// a quick tap shows "Hold to record". VoiceOver activates it to start and
/// again to send.
struct VoiceRecordButton: View {
    let chat: ChatModel
    let inThread: Bool
    var height: CGFloat = 42
    private var recorder: VoiceMessageRecorder { .shared }
    private var owner: VoiceComposerID { VoiceComposerID(chat: chat, inThread: inThread) }
    #if os(iOS)
    @State private var pressStart: Date?
    @State private var cancelled = false
    @GestureState private var pressing = false
    #endif

    var body: some View {
        let enabled = chat.canRecordVoice(inThread: inThread)
        let active = recorder.isActive(owner)
        #if os(iOS)
        icon(active: active)
            .opacity(enabled || active ? 1 : 0.45)
            .highPriorityGesture(holdGesture, including: enabled || active ? .all : .subviews)
            .onChange(of: pressing) { _, now in
                guard !now else { return }
                // The system cancelled the hold (such as the permission alert): discard.
                Task { @MainActor in
                    await Task.yield()
                    guard pressStart != nil else { return }
                    pressStart = nil; cancelled = false
                    if recorder.isActive(owner) { recorder.cancel() }
                }
            }
            .accessibilityElement()
            .accessibilityLabel(active ? "Send voice message" : "Record voice message")
            .accessibilityHint(active ? "" : "Hold to record, release to send, slide left to cancel.")
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { active ? send() : begin() }
            .accessibilityIdentifier(inThread ? "thread-voice-record" : "composer-voice-record")
            .disabled(!enabled && !active)
            .modifier(VoiceRecordingLifetime(owner: owner))
        #else
        Button(action: begin) { icon(active: active) }
            .buttonStyle(.plain)
            .disabled(!enabled)
            .help(enabled ? "Record voice message" : chat.canAttach ? "Send or dismiss the pending message first." : chat.attachUnavailableReason)
            .accessibilityLabel("Record voice message")
            .accessibilityIdentifier(inThread ? "thread-voice-record" : "composer-voice-record")
            .modifier(ControlPointer())
            .modifier(VoiceRecordingLifetime(owner: owner))
        #endif
    }

    private func icon(active: Bool) -> some View {
        CaperIcon(name: "mic", size: 18)
            .foregroundStyle(active ? CaperTheme.text : CaperTheme.muted)
            .frame(width: 30, height: 30)
            .background(Circle().fill(active ? CaperTheme.terracotta : .clear))
            .frame(width: 36, height: height)
            .contentShape(Rectangle())
    }

    private func begin() {
        guard chat.canRecordVoice(inThread: inThread) else { return }
        let chat = chat, inThread = inThread
        Task {
            await recorder.start(for: owner) {
                // 10:00: stop and send, on every platform.
                Task { @MainActor in await VoiceRecordButton.finishAndSend(chat: chat, inThread: inThread) }
            }
        }
    }

    private func send() {
        let chat = chat, inThread = inThread
        Task { await VoiceRecordButton.finishAndSend(chat: chat, inThread: inThread) }
    }

    @MainActor static func finishAndSend(chat: ChatModel, inThread: Bool) async {
        let recorder = VoiceMessageRecorder.shared
        let owner = VoiceComposerID(chat: chat, inThread: inThread)
        guard recorder.isRecording(owner) else { return }
        guard let recording = await recorder.finish() else {
            #if os(iOS)
            recorder.show(VoiceMessage.holdHint, for: owner, seconds: 2)
            #endif
            return
        }
        await chat.sendVoice(recording, inThread: inThread)
    }

    #if os(iOS)
    private var holdGesture: some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .local)
            .updating($pressing) { _, state, _ in state = true }
            .onChanged { value in
                if pressStart == nil {
                    pressStart = Date(); cancelled = false
                    begin()
                }
                guard !cancelled, recorder.isActive(owner) else { return }
                recorder.slide = min(0, Double(value.translation.width))
                if VoiceMessage.cancelsBySliding(translation: Double(value.translation.width)) {
                    cancelled = true
                    VoiceMessageRecorder.haptic()
                    recorder.cancel()
                }
            }
            .onEnded { _ in
                guard let start = pressStart else { return }
                pressStart = nil
                let wasCancelled = cancelled
                cancelled = false
                recorder.slide = 0
                // A start that failed (denied permission) already shows its notice.
                guard recorder.isActive(owner) else { return }
                let recorded = recorder.isRecording(owner) ? recorder.elapsedMs : 0
                switch VoiceMessage.release(pressDuration: Date().timeIntervalSince(start), recordedMs: recorded, cancelled: wasCancelled) {
                case .cancel: recorder.cancel()
                case .hint:
                    recorder.cancel()
                    recorder.show(VoiceMessage.holdHint, for: owner, seconds: 2)
                case .send: send()
                }
            }
    }
    #endif
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
/// Send; on iPhone and iPad the field beside the held microphone) is covered
/// by the recording bar. On the Mac it is also disabled, so typing and
/// Return can't reach the message field.
struct VoiceComposerCover: ViewModifier {
    let chat: ChatModel
    let inThread: Bool
    var height: CGFloat = 42
    /// Returns the keyboard to the message field once the bar closes (macOS).
    var restoreFocus: (() -> Void)? = nil
    private var recorder: VoiceMessageRecorder { .shared }

    func body(content: Content) -> some View {
        let recording = recorder.isRecording(VoiceComposerID(chat: chat, inThread: inThread))
        content
            .opacity(recording ? 0 : 1)
            .allowsHitTesting(!recording)
            #if os(macOS)
            // Gives up keyboard focus, so Return reaches the bar's Send rather than the message field.
            .disabled(recording)
            #endif
            // iPhone keeps the keyboard up under a held recording, so the composer doesn't move.
            .accessibilityHidden(recording)
            .overlay(alignment: .bottom) {
                if recording {
                    VoiceRecordingBar(chat: chat, inThread: inThread, height: height)
                        .modifier(Entrance.anchored)
                }
            }
            .onChange(of: recording) { _, now in
                if !now { restoreFocus?() }
            }
    }
}

/// `[trash]  ● 0:07  live bars  [send]` on the Mac (Escape discards, Return
/// sends); on touch devices the timer, live bars and "Slide to cancel", with
/// "Release to send" above (`VoiceComposerNotice`).
struct VoiceRecordingBar: View {
    let chat: ChatModel
    let inThread: Bool
    var height: CGFloat = 42
    private var recorder: VoiceMessageRecorder { .shared }

    var body: some View {
        #if os(iOS)
        let progress = VoiceMessage.slideProgress(translation: recorder.slide)
        HStack(spacing: 10) {
            timer
            VoiceWaveformBars(source: .live(recorder.levels), playedFraction: 1)
                .frame(height: 22)
                .frame(maxWidth: .infinity)
            HStack(spacing: 4) {
                CaperIcon(name: "chevron-left", size: 14)
                Text(VoiceMessage.slideHint).font(CaperTheme.font(12)).lineLimit(1)
            }
            .foregroundStyle(CaperTheme.muted)
            .opacity(1 - progress * 0.8)
            // Follows the finger (not an animation), so Reduce Motion is unaffected.
            .offset(x: CGFloat(max(-VoiceMessage.cancelDistance, recorder.slide)) * 0.5)
        }
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity, minHeight: height, maxHeight: height)
        .background(CaperTheme.composer)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Recording voice message, \(VoiceDuration.format(ms: recorder.elapsedMs)). Release to send, slide left to cancel.")
        #else
        HStack(spacing: 10) {
            Button { recorder.cancel() } label: {
                CaperIcon(name: "trash-2", size: 16).foregroundStyle(CaperTheme.muted)
                    .frame(width: 34, height: 34).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .keyboardShortcut(.cancelAction)
            .help("Discard voice message")
            .accessibilityLabel("Discard voice message")
            .modifier(ControlPointer())
            timer
            VoiceWaveformBars(source: .live(recorder.levels), playedFraction: 1)
                .frame(height: 22)
                .frame(maxWidth: .infinity)
            Button {
                let chat = chat, inThread = inThread
                Task { await VoiceRecordButton.finishAndSend(chat: chat, inThread: inThread) }
            } label: {
                CaperIcon(name: "send", size: 16)
                    .frame(width: 42, height: height)
                    .contentShape(Rectangle())
            }
            .buttonStyle(PrimaryIconButton())
            .keyboardShortcut(.defaultAction)
            .help("Send voice message")
            .accessibilityLabel("Send voice message")
        }
        .padding(.leading, 4)
        .frame(maxWidth: .infinity, minHeight: height, maxHeight: height)
        .background(CaperTheme.composer)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Recording voice message")
        .accessibilityIdentifier(inThread ? "thread-voice-recording" : "composer-voice-recording")
        #endif
    }

    private var timer: some View {
        HStack(spacing: 6) {
            Circle().fill(CaperTheme.terracottaBright).frame(width: 8, height: 8)
            Text(VoiceDuration.format(ms: recorder.elapsedMs))
                .font(CaperTheme.font(13, weight: .medium)).monospacedDigit().foregroundStyle(CaperTheme.text)
        }
        .fixedSize()
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Recording, \(VoiceDuration.format(ms: recorder.elapsedMs))")
    }
}

/// The microphone's inline message for one composer (denied permission,
/// "Hold to record", a failed recording), and on touch devices "Release to
/// send" above a held recording.
struct VoiceComposerNotice: View {
    let chat: ChatModel
    let inThread: Bool
    private var recorder: VoiceMessageRecorder { .shared }

    var body: some View {
        let owner = VoiceComposerID(chat: chat, inThread: inThread)
        #if os(iOS)
        if recorder.isRecording(owner) {
            Text(VoiceMessage.releaseHint)
                .font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.text)
                .frame(maxWidth: .infinity)
                .accessibilityHidden(true)
        }
        #endif
        if let text = recorder.notice(for: owner) {
            Text(text)
                .font(CaperTheme.font(11))
                .foregroundStyle(text == VoiceMessage.holdHint ? CaperTheme.muted : CaperTheme.error)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier(inThread ? "thread-voice-notice" : "composer-voice-notice")
        }
    }
}
