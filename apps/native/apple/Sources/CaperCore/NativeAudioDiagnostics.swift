import Foundation

enum NativeAudioReport {
    enum Kind { case connection, processing }
    @MainActor static func statistics(_ voice: VoiceClient) -> VoiceDiagnostics? {
        if CaperRuntime.isAudioPreview("audio-statistics") {
            return VoiceDiagnostics(receivedBytes: 65_432, sentBytes: 12_345, receiveBitrate: 12_800, sendBitrate: 24_000,
                packetsLost: 3, maxJitterMs: 17, roundTripMs: 42, route: "relay", checks: "4 sent · 4 answered",
                timing: VoiceJoinTiming(joinedMs: 812, sessionMs: 214, transportMs: 391, rosterMs: 88))
        }
        return voice.diagnostics
    }
    static func rows(_ stats: VoiceDiagnostics) -> [(String, String)] {
        func mb(_ bytes: Int64) -> String { String(format: "%.2f MB", Double(bytes) / 1e6) }
        func kbps(_ value: Int?) -> String { value.map { "\(Int((Double($0) / 1_000).rounded())) kbps" } ?? "Not observed yet" }
        func ms(_ value: Int?) -> String { value.map { "\($0) ms" } ?? "Not observed yet" }
        let timing: [(String, String)] = stats.timing.map { [
            ("Joined", "Joined in \($0.joinedMs) ms"), ("Session + publish", "\($0.sessionMs) ms"),
            ("Transport + state", "\($0.transportMs) ms"), ("Connectivity checks", stats.checks ?? "Not observed yet"), ("Roster", "\($0.rosterMs) ms")
        ] } ?? []
        return timing + [("Received", mb(stats.receivedBytes)), ("Live receive", kbps(stats.receiveBitrate)),
            ("Sent", mb(stats.sentBytes)), ("Live send", kbps(stats.sendBitrate)), ("Packets lost", String(stats.packetsLost)),
            ("Max jitter", ms(stats.maxJitterMs)), ("RTT", ms(stats.roundTripMs)),
            ("Route", stats.route == "relay" ? "TURN relay" : stats.route == "direct" ? "Direct" : "Not observed yet")]
    }
    static func connection(_ stats: VoiceDiagnostics) -> String {
        var values: [String: Any] = ["receivedBytes": stats.receivedBytes, "sentBytes": stats.sentBytes,
            "receiveBitrate": stats.receiveBitrate ?? 0, "sendBitrate": stats.sendBitrate ?? 0, "packetsLost": stats.packetsLost,
            "maxJitterMs": stats.maxJitterMs ?? 0, "roundTripMs": stats.roundTripMs ?? 0, "route": stats.route]
        if let timing = stats.timing {
            values["join"] = "Joined in \(timing.joinedMs) ms"; values["sessionMs"] = timing.sessionMs
            values["transportMs"] = timing.transportMs; values["rosterMs"] = timing.rosterMs
            if let checks = stats.checks { values["checks"] = checks }
        }
        return json(values)
    }
    @MainActor static func processing(_ voice: VoiceClient) -> String {
        #if os(macOS)
        let platform = "macOS"
        #else
        let platform = "iOS"
        #endif
        var values: [String: Any] = ["platform": platform, "processing": NSNull()]
        if let stats = voice.audioProcessingReport {
            values["processing"] = ["mode": stats.mode, "processedHops": stats.processedHops, "meanProcessingMs": stats.meanProcessingMs,
                "maxProcessingMs": stats.maxProcessingMs, "queuedInputMs": stats.queuedInputMs, "hopBudgetMs": 10] as [String: Any]
        }
        return json(values)
    }
    private static func json(_ values: [String: Any]) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: values, options: [.prettyPrinted, .sortedKeys]),
              let text = String(data: data, encoding: .utf8) else { return "Diagnostics unavailable." }; return text
    }
}

#if os(macOS)
import AppKit

@MainActor final class AppKitAudioDiagnostics: AppKitSettingsDialog {
    private let voice: VoiceClient
    private let kind: NativeAudioReport.Kind
    private let rows = AppKitKit.stack([], spacing: 6)
    private let report = AppKitKit.label("", size: 11)
    private let status = AppKitKit.label("", size: 11, color: AppKitKit.muted)
    private var copy: AppKitKitButton!
    private var timer: Timer?
    private var refresh: Task<Void, Never>?
    private var text = ""
    private var previousRows: [String] = []
    init(voice: VoiceClient, kind: NativeAudioReport.Kind, close: @escaping () -> Void) {
        self.voice = voice; self.kind = kind
        super.init(title: kind == .connection ? "Connection details" : "Audio diagnostics", detail: kind == .connection ? "" : "Local processing counters",
            closeLabel: kind == .connection ? "Close connection details" : "Close audio diagnostics", identifier: kind == .connection ? "connection-details" : "audio-diagnostics", close: close)
        copy = AppKitKitButton(kind == .connection ? "Copy connection details" : "Copy diagnostics") { [weak self] in
            guard let self else { return }; NSPasteboard.general.clearContents()
            self.status.stringValue = NSPasteboard.general.setString(self.text, forType: .string)
                ? (self.kind == .connection ? "Copied connection details" : "Copied diagnostics") : "Copy failed; select the report below."
        }
        report.font = .monospacedSystemFont(ofSize: 11, weight: .regular); report.isSelectable = true
        setRows([rows, copy, status, report]); if kind == .processing { preferHeight(560) }
        observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { timer?.invalidate(); refresh?.cancel() }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow(); timer?.invalidate(); timer = nil
        if window != nil { tick(); timer = Timer.scheduledTimer(withTimeInterval: kind == .connection ? 2 : 1, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.tick() } } }
        else { refresh?.cancel(); refresh = nil }
    }
    private func tick() {
        render()
        if kind == .connection, voice.phase == .connected, refresh == nil {
            refresh = Task { [weak self, voice] in await voice.refreshDiagnostics(); self?.refresh = nil }
        }
    }
    private func render() {
        var lines: [(String, String)] = []
        if kind == .connection {
            if CaperRuntime.isAudioPreview("audio-statistics") { lines.append(("", "TEST FIXTURE — synthetic statistics layout; no voice connection.")) }
            if let stats = NativeAudioReport.statistics(voice) {
                lines += NativeAudioReport.rows(stats); lines.append(("", "Counters reset on reconnect.")); text = NativeAudioReport.connection(stats)
                copy.isHidden = false
            } else { lines.append(("", voice.phase == .connected ? "Waiting for transport statistics…" : "Join voice to see connection details.")); copy.isHidden = true }
            report.isHidden = true
        } else {
            lines.append(("", "Local diagnostics only. No audio, device identifiers, or credentials. Nothing is uploaded."))
            if voice.phase != .connected { lines.append(("", "No microphone capture started. Open Mic Test or join voice first.")) }
            lines.append(("", voice.noiseSuppressionStatus)); text = NativeAudioReport.processing(voice); report.stringValue = text
        }
        let key = lines.map { $0.0 + "\n" + $0.1 }; guard key != previousRows else { return }; previousRows = key
        rows.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for (title, value) in lines {
            let row: NSView
            if title.isEmpty { row = AppKitKit.label(value, size: 12, color: AppKitKit.muted) }
            else {
                let spacer = NSView(); spacer.setContentHuggingPriority(.init(1), for: .horizontal)
                let stack = NSStackView(views: [AppKitKit.label(title, size: 13, weight: .bold), spacer, AppKitKit.label(value, size: 13, color: AppKitKit.muted)])
                stack.spacing = 10; row = stack
            }
            rows.addArrangedSubview(row); AppKitKit.fillWidth(row, of: rows)
        }
    }
}
#elseif os(iOS)
import UIKit

@MainActor final class UIKitAudioDiagnostics: UIKitMessageDialog {
    private let voice: VoiceClient
    private let kind: NativeAudioReport.Kind
    private let rows = UIStackView()
    private let report = UITextView()
    private let status = UIKitMessageDialog.label("")
    private var copy: UIButton!
    private var timer: Timer?
    private var refresh: Task<Void, Never>?
    private var text = ""
    private var previousRows: [String] = []
    init(voice: VoiceClient, kind: NativeAudioReport.Kind, close: @escaping () -> Void) {
        self.voice = voice; self.kind = kind
        super.init(title: kind == .connection ? "Connection details" : "Audio diagnostics",
            closeLabel: kind == .connection ? "Close connection details" : "Close audio diagnostics", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { timer?.invalidate(); refresh?.cancel() }
    override func viewDidLoad() {
        super.viewDidLoad(); rows.axis = .vertical; rows.spacing = 6
        view.accessibilityIdentifier = kind == .connection ? "connection-details" : "audio-diagnostics"
        copy = Self.button(kind == .connection ? "Copy connection details" : "Copy diagnostics") { [weak self] in
            guard let self else { return }; UIPasteboard.general.string = self.text
            self.status.text = self.kind == .connection ? "Copied connection details" : "Copied diagnostics"
        }
        report.font = .monospacedSystemFont(ofSize: 11, weight: .regular); report.textColor = UIColor(CaperTheme.text)
        report.backgroundColor = .clear; report.isEditable = false; report.isScrollEnabled = false
        [rows, copy!, status, report].forEach(body.addArrangedSubview); observe()
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated); tick(); timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: kind == .connection ? 2 : 1, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.tick() } }
    }
    override func stop() { super.stop(); timer?.invalidate(); timer = nil; refresh?.cancel(); refresh = nil }
    private func tick() {
        render()
        if kind == .connection, voice.phase == .connected, refresh == nil {
            refresh = Task { [weak self, voice] in await voice.refreshDiagnostics(); self?.refresh = nil }
        }
    }
    override func render() {
        var lines: [(String, String)] = []
        if kind == .connection {
            if CaperRuntime.isAudioPreview("audio-statistics") { lines.append(("", "TEST FIXTURE — synthetic statistics layout; no voice connection.")) }
            if let stats = NativeAudioReport.statistics(voice) {
                lines += NativeAudioReport.rows(stats); lines.append(("", "Counters reset on reconnect.")); text = NativeAudioReport.connection(stats); copy.isHidden = false
            } else { lines.append(("", voice.phase == .connected ? "Waiting for transport statistics…" : "Join voice to see connection details.")); copy.isHidden = true }
            report.isHidden = true
        } else {
            lines.append(("", "Local diagnostics only. No audio, device identifiers, or credentials. Nothing is uploaded."))
            if voice.phase != .connected { lines.append(("", "No microphone capture started. Open Mic Test or join voice first.")) }
            lines.append(("", voice.noiseSuppressionStatus)); text = NativeAudioReport.processing(voice); report.text = text
        }
        let key = lines.map { $0.0 + "\n" + $0.1 }; guard key != previousRows else { return }; previousRows = key
        rows.arrangedSubviews.forEach { rows.removeArrangedSubview($0); $0.removeFromSuperview() }
        for (title, value) in lines {
            if title.isEmpty { rows.addArrangedSubview(Self.label(value)) }
            else {
                let label = Self.label(value, size: 13); label.textAlignment = .right
                let row = UIStackView(arrangedSubviews: [Self.label(title, size: 13, bold: true), label]); row.spacing = 10; rows.addArrangedSubview(row)
            }
        }
        parent?.view.setNeedsLayout()
    }
}
#endif
