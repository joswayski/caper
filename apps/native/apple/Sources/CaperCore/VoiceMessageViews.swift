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
/// Played bars (the boundary one partly) draw in the text colour. Live bars
/// scroll right to left by `scroll` of one step between new readings.
struct VoiceWaveformBars: View {
    enum Source: Equatable {
        /// A sent note's waveform bytes (nil: flat line), linearly resampled to fit.
        case waveform([UInt8]?)
        /// Live recording levels, newest at the right.
        case live([Float])
    }
    let source: Source
    var playedFraction: Double = 0
    /// Live bars only: 0...1 of a step travelled since the newest bar entered.
    var scroll: Double = 1
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
            case .live(let levels):
                // One extra bar enters from beyond the right edge as the bars slide left.
                for (index, value) in VoiceWaveform.liveBars(levels, count: count + 1).enumerated() {
                    let x = (CGFloat(index) - CGFloat(scroll)) * step
                    draw(&context, bar(x: x, value: value, size: size), fill: 1)
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
/// transcript"; "Transcribing…" while pending; nothing when failed, absent,
/// unknown or empty. The text is selectable.
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

    @MainActor static func chat(_ chat: ChatModel, inThread: Bool) -> VoiceTarget {
        let sendsOnStop = VoiceMessageSettings.shared.sendOnStop
        let reason: String
        if !chat.canAttach { reason = chat.attachUnavailableReason }
        else if chat.attachmentFiles(inThread: inThread).contains(where: \.isVoice) { reason = VoiceMessage.oneVoicePerMessage }
        else if sendsOnStop { reason = "Send or dismiss the pending message first." }
        else { reason = "You can attach up to \(AttachmentPolicy.maxAttachments) files." }
        return VoiceTarget(owner: VoiceComposerID(chat: chat, inThread: inThread),
                           enabled: sendsOnStop ? chat.canRecordVoice(inThread: inThread) : chat.canStageVoice(inThread: inThread),
                           unavailableReason: reason,
                           identifier: inThread ? "thread-voice" : "composer-voice",
                           send: { await chat.sendVoice($0, inThread: inThread) },
                           stage: { chat.stageVoice($0, inThread: inThread) })
    }

    /// Stop (or a released hold, or the 10:00 limit): stage or send.
    @MainActor static func finish(_ target: VoiceTarget) async {
        let recorder = VoiceMessageRecorder.shared
        guard recorder.isRecording(target.owner), let recording = await recorder.finish() else { return }
        switch VoiceMessage.finish(sendsOnStop: VoiceMessageSettings.shared.sendOnStop) {
        case .send: await target.send(recording)
        case .stage: target.stage(recording)
        }
    }
}

/// The composer's microphone ("Record voice message"), a tool button beside
/// Emoji. A click or tap records hands-free until Stop: on the Mac the row
/// becomes the recording bar with Discard and Stop; on iPhone and iPad the
/// microphone itself becomes Stop. Touch layouts can also press and hold
/// (release to stop, slide left to cancel). VoiceOver activates it to start
/// and again to stop.
struct VoiceRecordButton: View {
    let target: VoiceTarget
    var height: CGFloat = 42
    private var recorder: VoiceMessageRecorder { .shared }
    private var owner: VoiceComposerID { target.owner }
    #if os(iOS)
    @State private var pressStart: Date?
    @State private var cancelled = false
    @GestureState private var pressing = false
    #endif

    init(target: VoiceTarget, height: CGFloat = 42) {
        self.target = target
        self.height = height
    }

    init(chat: ChatModel, inThread: Bool, height: CGFloat = 42) {
        self.init(target: .chat(chat, inThread: inThread), height: height)
    }

    var body: some View {
        ZStack {
            #if os(iOS)
            if recorder.isActive(owner) && recorder.handsFree {
                VoiceStopButton(height: height) { stop() }
                    .accessibilityIdentifier("\(target.identifier)-stop")
            } else {
                holdableMicrophone
            }
            #else
            Button { begin(handsFree: true) } label: { microphone }
                .buttonStyle(.plain)
                .disabled(!target.enabled)
                .help(target.enabled ? VoiceMessage.recordLabel : target.unavailableReason)
                .accessibilityLabel(VoiceMessage.recordLabel)
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
        guard target.enabled else { return }
        let target = target
        Task {
            await VoiceMessageRecorder.shared.start(for: target.owner, handsFree: handsFree) {
                // 10:00: stop, then stage or send, on every platform.
                Task { @MainActor in await VoiceTarget.finish(target) }
            }
        }
    }

    private func stop() {
        let target = target
        Task { await VoiceTarget.finish(target) }
    }

    #if os(iOS)
    private var holdableMicrophone: some View {
        let enabled = target.enabled
        let active = recorder.isActive(owner)
        return microphone
            .opacity(enabled || active ? 1 : 0.45)
            .highPriorityGesture(holdGesture, including: enabled || active ? .all : .subviews)
            .onChange(of: pressing) { _, now in
                guard !now else { return }
                // The system took the touch (such as the permission alert): keep
                // recording hands-free rather than losing it; Stop or Discard ends it.
                Task { @MainActor in
                    await Task.yield()
                    guard pressStart != nil else { return }
                    pressStart = nil; cancelled = false
                    if recorder.isActive(owner) { recorder.handsFree = true; recorder.slide = 0 }
                }
            }
            .accessibilityElement()
            .accessibilityLabel(VoiceMessage.recordLabel)
            .accessibilityHint(enabled ? "Records until you stop. You can also hold to record and release to stop." : target.unavailableReason)
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
                    pressStart = Date(); cancelled = false
                    begin(handsFree: false)
                }
                guard !cancelled, recorder.isActive(owner), !recorder.handsFree else { return }
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
                case .cancel, .discard: recorder.cancel()
                case .handsFree: recorder.handsFree = true
                case .stop: stop()
                }
            }
    }
    #endif
}

/// The eggshell square Stop of a hands-free recording ("Stop recording").
struct VoiceStopButton: View {
    var height: CGFloat = 42
    var width: CGFloat = 36
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "stop.fill").font(.system(size: 12, weight: .bold))
                .frame(width: 30, height: 30)
                .frame(width: width, height: height)
                .contentShape(Rectangle())
        }
        .buttonStyle(VoiceStopStyle())
        .help(VoiceMessage.stopLabel)
        .accessibilityLabel(VoiceMessage.stopLabel)
    }
}

/// Eggshell fill with a blackout glyph, as primary actions, on the glyph's square only.
private struct VoiceStopStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(CaperTheme.onAction)
            .background {
                RoundedRectangle(cornerRadius: 8)
                    .fill(configuration.isPressed ? CaperTheme.actionPress : CaperTheme.action)
                    .frame(width: 30, height: 30)
            }
            .modifier(ControlPointer())
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

    init(chat: ChatModel, inThread: Bool, height: CGFloat = 42, restoreFocus: (() -> Void)? = nil) {
        self.init(target: .chat(chat, inThread: inThread), height: height, restoreFocus: restoreFocus)
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
            // iPhone keeps the keyboard up under a recording, so the composer doesn't move.
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

/// The recording bar: `[trash]  ● 0:07  live bars  [Stop]` on the Mac
/// (Return stops, Escape discards). On touch devices a held recording shows
/// the timer, live bars and "Slide to cancel"; a hands-free one shows Discard,
/// the timer and live bars, with the microphone beside it turned into Stop.
struct VoiceRecordingBar: View {
    let target: VoiceTarget
    var height: CGFloat = 42
    private var recorder: VoiceMessageRecorder { .shared }

    var body: some View {
        #if os(iOS)
        HStack(spacing: 10) {
            if recorder.handsFree { discard } else { Color.clear.frame(width: 2) }
            VoiceRecordingTimer(elapsedMs: recorder.elapsedMs)
            VoiceLiveBars()
            if !recorder.handsFree { slideHint }
        }
        .padding(.trailing, 12)
        .frame(maxWidth: .infinity, minHeight: height, maxHeight: height)
        .background(CaperTheme.composer)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Recording voice message")
        .accessibilityIdentifier("\(target.identifier)-recording")
        #else
        HStack(spacing: 10) {
            discard.keyboardShortcut(.cancelAction)
            VoiceRecordingTimer(elapsedMs: recorder.elapsedMs)
            VoiceLiveBars()
            Button {
                let target = target
                Task { await VoiceTarget.finish(target) }
            } label: {
                Image(systemName: "stop.fill").font(.system(size: 13, weight: .bold))
                    .frame(width: 42, height: height)
                    .contentShape(Rectangle())
            }
            .buttonStyle(PrimaryIconButton())
            .keyboardShortcut(.defaultAction)
            .help("\(VoiceMessage.stopLabel) (Return)")
            .accessibilityLabel(VoiceMessage.stopLabel)
            .accessibilityIdentifier("\(target.identifier)-stop")
        }
        .padding(.leading, 4)
        .frame(maxWidth: .infinity, minHeight: height, maxHeight: height)
        .background(CaperTheme.composer)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Recording voice message")
        .accessibilityIdentifier("\(target.identifier)-recording")
        #endif
    }

    private var discard: some View {
        Button { recorder.cancel() } label: {
            CaperIcon(name: "trash-2", size: 16).foregroundStyle(CaperTheme.muted)
                .frame(width: 36, height: 36).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(VoiceMessage.discardLabel)
        .accessibilityLabel(VoiceMessage.discardLabel)
        .accessibilityIdentifier("\(target.identifier)-discard")
        .modifier(ControlPointer())
    }

    #if os(iOS)
    private var slideHint: some View {
        let progress = VoiceMessage.slideProgress(translation: recorder.slide)
        return HStack(spacing: 4) {
            CaperIcon(name: "chevron-left", size: 14)
            Text(VoiceMessage.slideHint).font(CaperTheme.font(12)).lineLimit(1)
        }
        .foregroundStyle(CaperTheme.muted)
        .opacity(1 - progress * 0.8)
        // Follows the finger (not an animation), so Reduce Motion is unaffected.
        .offset(x: CGFloat(max(-VoiceMessage.cancelDistance, recorder.slide)) * 0.5)
    }
    #endif
}

/// The recording dot (pulsing; still under Reduce Motion) and m:ss.
struct VoiceRecordingTimer: View {
    let elapsedMs: Int
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 6) {
            TimelineView(.animation(paused: reduceMotion)) { context in
                let phase = reduceMotion ? 1 : 0.55 + 0.45 * (0.5 + 0.5 * cos(context.date.timeIntervalSinceReferenceDate * .pi))
                Circle().fill(CaperTheme.terracottaBright).frame(width: 8, height: 8).opacity(phase)
            }
            .frame(width: 8, height: 8)
            Text(VoiceDuration.format(ms: elapsedMs))
                .font(CaperTheme.font(13, weight: .medium)).monospacedDigit().foregroundStyle(CaperTheme.text)
        }
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Recording, \(VoiceDuration.format(ms: elapsedMs))")
    }
}

/// The live input bars, scrolling right to left at a constant rate (new bars
/// step in without the slide under Reduce Motion).
struct VoiceLiveBars: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var recorder: VoiceMessageRecorder { .shared }

    var body: some View {
        let levels = recorder.levels
        TimelineView(.animation(paused: reduceMotion)) { context in
            VoiceWaveformBars(source: .live(levels),
                              scroll: reduceMotion ? 1 : VoiceWaveform.scrollFraction(sinceLastBar: context.date.timeIntervalSince(recorder.lastBarAt)))
        }
        .frame(height: 22)
        .frame(maxWidth: .infinity)
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
        if recorder.isRecording(owner) && !recorder.handsFree {
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
