// NEVER MERGE: standalone AppKit/AX discriminator, no Caper or SwiftUI linkage.
import AppKit

@main @MainActor
final class AXMenuControl: NSObject, NSApplicationDelegate {
    private var window: NSWindow?

    static func main() {
        let application = NSApplication.shared
        let delegate = AXMenuControl()
        application.delegate = delegate
        application.setActivationPolicy(.regular)
        withExtendedLifetime(delegate) { application.run() }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 400),
            styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "Standalone AppKit AX diagnostic"
        let field = NSTextField(string: "Standalone text field")
        field.frame = NSRect(x: 24, y: 180, width: 400, height: 28)
        field.setAccessibilityIdentifier("standalone-text-field")
        window.contentView?.addSubview(field)
        self.window = window
        // The same ordinary App/File/Edit menu categories and responder actions
        // as the pinned host, without model-dependent callbacks or dependencies.
        let menu = NSMenu()
        let app = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "Settings…", action: #selector(diagnosticAction(_:)), keyEquivalent: ",").target = self
        appMenu.addItem(withTitle: "What’s new", action: #selector(diagnosticAction(_:)), keyEquivalent: "").target = self
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Quit AX diagnostic", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        app.submenu = appMenu; menu.addItem(app)
        let file = NSMenuItem()
        let fileMenu = NSMenu(title: "File")
        fileMenu.addItem(withTitle: "Close Window", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        file.submenu = fileMenu; menu.addItem(file)
        let edit = NSMenuItem()
        let editMenu = NSMenu(title: "Edit")
        for (title, action, key) in [("Undo", "undo:", "z"), ("Cut", "cut:", "x"), ("Copy", "copy:", "c"),
                                    ("Paste", "paste:", "v"), ("Select All", "selectAll:", "a")] {
            editMenu.addItem(withTitle: title, action: Selector(action), keyEquivalent: key)
        }
        editMenu.addItem(.separator())
        editMenu.addItem(withTitle: "B — Bold", action: #selector(diagnosticAction(_:)), keyEquivalent: "b").target = self
        editMenu.addItem(withTitle: "I — Italic", action: #selector(diagnosticAction(_:)), keyEquivalent: "i").target = self
        edit.submenu = editMenu; menu.addItem(edit)
        NSApp.mainMenu = menu
        if let path = ProcessInfo.processInfo.environment["AX_MENU_PID_PATH"] {
            do { try String(ProcessInfo.processInfo.processIdentifier).write(toFile: path, atomically: true, encoding: .utf8) }
            catch { fatalError("Cannot record diagnostic PID: \(error)") }
        }
        // Ordinary app startup only, matching the host; no test click or focus workaround.
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    // No menu item is invoked by the test; local callbacks replace product-only actions.
    @objc private func diagnosticAction(_ sender: Any?) {}
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
