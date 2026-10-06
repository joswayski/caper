import AppKit
import Foundation

/// Self-updates through `caper-updater` (apps/native/updater), which release
/// builds carry in Contents/MacOS. It checks the signed release manifest shortly
/// after launch and every minute; when the user accepts, the updater waits
/// for Caper to quit, swaps in the new notarized app and reopens it.
///
/// Prompts are AppKit alerts rather than SwiftUI `.alert` modifiers, so they
/// never compete with the confirmation dialogs inside the Caper window.
@MainActor final class AppUpdater {
    struct Update: Equatable {
        let version: String
        let notes: String
        let canApply: Bool
    }

    private let updater: URL?
    private let build: Int
    private var declined: Set<String> = []
    private var timer: Timer?
    private var checking = false
    private var manualCheck = false

    static let firstCheckDelay: TimeInterval = 20
    static let checkInterval: TimeInterval = 60

    // Match the running app's architecture, including Intel builds under Rosetta.
    #if arch(arm64)
    static let downloadURL = URL(string: "https://github.com/joswayski/caper/releases/download/native-latest/Caper-macOS-Apple-Silicon.dmg")!
    #else
    static let downloadURL = URL(string: "https://github.com/joswayski/caper/releases/download/native-latest/Caper-macOS-Intel.dmg")!
    #endif

    init(bundle: Bundle = .main, environment: [String: String] = ProcessInfo.processInfo.environment) {
        let updater = bundle.bundleURL.appendingPathComponent("Contents/MacOS/caper-updater")
        let build = Int(bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "") ?? 0
        // Only signed release builds ship the updater; tests never check.
        let enabled = environment["CAPER_TEST_MODE"] == nil
            && build > 0
            && FileManager.default.isExecutableFile(atPath: updater.path)
        self.updater = enabled ? updater : nil
        self.build = build
    }

    var isAvailable: Bool { updater != nil }

    /// Signed downloads must be installed before opening the account UI. Finder
    /// owns the move and Gatekeeper handling; never strip quarantine or guess
    /// the original path of an app running from a translocated read-only copy.
    func prepareInstallation() -> Bool {
        guard isAvailable else { return true } // Development and UI fixtures.
        let manager = FileManager.default
        let bundle = Bundle.main.bundleURL
        // Like caper-updater's eligibility check, test an actual write in the
        // parent that must hold the staged replacement, not just permission bits.
        let probe = bundle.deletingLastPathComponent()
            .appendingPathComponent(".caper-install-probe-\(UUID().uuidString)")
        let parentWritable = manager.createFile(atPath: probe.path, contents: Data())
        if parentWritable { try? manager.removeItem(at: probe) }
        guard !Self.needsInstallation(bundle, home: manager.homeDirectoryForCurrentUser,
                                      parentWritable: parentWritable) else {
            let alert = NSAlert()
            alert.messageText = "Install Caper before opening it"
            alert.informativeText = "Open the Caper disk image and drag Caper onto Applications. Eject the disk image, then open Caper from Applications. This keeps you on the installed copy that can receive updates.\n\nIf Applications requires administrator access, use your home folder's Applications instead."
            alert.addButton(withTitle: "Open Applications")
            alert.addButton(withTitle: "Download Installer")
            alert.addButton(withTitle: "Quit")
            NSApp.activate(ignoringOtherApps: true)
            switch alert.runModal() {
            case .alertFirstButtonReturn:
                var destination = URL(fileURLWithPath: "/Applications", isDirectory: true)
                if !manager.isWritableFile(atPath: destination.path) {
                    destination = manager.homeDirectoryForCurrentUser.appendingPathComponent("Applications", isDirectory: true)
                    do {
                        try manager.createDirectory(at: destination, withIntermediateDirectories: true)
                    } catch {
                        inform("Could not open Applications", error.localizedDescription)
                        NSApp.terminate(nil)
                        return false
                    }
                }
                NSWorkspace.shared.open(destination)
            case .alertSecondButtonReturn:
                NSWorkspace.shared.open(Self.downloadURL)
            default:
                break
            }
            NSApp.terminate(nil)
            return false
        }
        return true
    }

    nonisolated static func needsInstallation(_ bundle: URL, home: URL, parentWritable: Bool) -> Bool {
        let path = bundle.standardizedFileURL.resolvingSymlinksInPath().pathComponents
        let locations = [URL(fileURLWithPath: "/Applications", isDirectory: true),
                         home.appendingPathComponent("Applications", isDirectory: true)]
        let installed = locations.contains { location in
            let root = location.standardizedFileURL.resolvingSymlinksInPath().pathComponents
            return path.count > root.count && path.starts(with: root)
        }
        return !installed || !parentWritable
    }

    func start() {
        guard isAvailable, timer == nil else { return }
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(Self.firstCheckDelay))
            self?.check(manual: false)
        }
        timer = Timer.scheduledTimer(withTimeInterval: Self.checkInterval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.check(manual: false) }
        }
    }

    func check(manual: Bool) {
        guard let updater else { return }
        // A manual request can share a running automatic check, but must still
        // report its result and offer a version previously declined with Later.
        manualCheck = manualCheck || manual
        guard !checking else { return }
        checking = true
        let arguments = ["check", "--current-build", String(build), "--install", Bundle.main.bundleURL.path]
        Task.detached(priority: .utility) {
            let result = Self.run(updater, arguments)
            await MainActor.run { self.finish(result) }
        }
    }

    private func finish(_ result: Result<Data, Error>) {
        let manual = manualCheck
        manualCheck = false
        // Keep the check fenced while a modal prompt runs its nested event loop.
        defer {
            checking = false
            manualCheck = false
        }
        switch result {
        case let .success(data):
            guard let update = Self.parse(data) else {
                if manual { inform("Caper is up to date", "") }
                return
            }
            if manual || !declined.contains(update.version) { offer(update) }
        case let .failure(error):
            if manual { inform("Update failed", error.localizedDescription) }
        }
    }

    private func offer(_ update: Update) {
        let alert = NSAlert()
        alert.messageText = "Caper \(update.version) is available"
        let detail = update.canApply
            ? "Caper restarts to install it and leaves any voice call."
            : "Caper can't update this copy in place. Download the installer and drag Caper into a writable Applications folder."
        alert.informativeText = update.notes.isEmpty ? detail : "\(update.notes)\n\n\(detail)"
        alert.addButton(withTitle: update.canApply ? "Restart to Update" : "Download")
        alert.addButton(withTitle: "Later")
        NSApp.activate(ignoringOtherApps: true)
        guard alert.runModal() == .alertFirstButtonReturn else {
            declined.insert(update.version)
            return
        }
        if update.canApply {
            restart()
        } else {
            NSWorkspace.shared.open(Self.downloadURL)
        }
    }

    private func inform(_ title: String, _ detail: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = detail
        alert.runModal()
    }

    /// Starts the updater, which waits for this process to exit, then quits.
    private func restart() {
        guard let updater else { return }
        let process = Process()
        process.executableURL = updater
        process.arguments = [
            "apply", "--current-build", String(build),
            "--install", Bundle.main.bundleURL.path,
            "--wait-pid", String(ProcessInfo.processInfo.processIdentifier),
        ]
        do {
            try process.run()
            NSApp.terminate(nil)
        } catch {
            inform("Update failed", "Could not start the update: \(error.localizedDescription)")
        }
    }

    nonisolated static func parse(_ data: Data) -> Update? {
        struct Output: Decodable {
            let update: Bool
            let version: String?
            let notes: String?
            let can_apply: Bool?
        }
        guard let output = try? JSONDecoder().decode(Output.self, from: data), output.update else { return nil }
        return Update(version: output.version ?? "", notes: output.notes ?? "", canApply: output.can_apply ?? false)
    }

    private nonisolated static func run(_ executable: URL, _ arguments: [String]) -> Result<Data, Error> {
        let process = Process()
        let output = Pipe()
        process.executableURL = executable
        process.arguments = arguments
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            let data = output.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            guard process.terminationStatus == 0 else {
                return .failure(UpdateCheckError())
            }
            return .success(data)
        } catch {
            return .failure(error)
        }
    }
}

private struct UpdateCheckError: LocalizedError {
    var errorDescription: String? { "Caper could not check for updates. Try again later." }
}
