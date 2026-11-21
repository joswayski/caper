import XCTest
@testable import CaperCore
#if os(macOS)
import AppKit
#endif

final class CaperMenuTests: XCTestCase {
    private func shape(_ items: [CaperMenuItem]) -> [String] {
        items.map { item in
            switch item.kind {
            case .separator: "—"
            case let .submenu(children): "\(item.title)[\(shape(children).joined(separator: ","))]"
            case .note: "(\(item.title))"
            case .action: item.title
            }
        }
    }

    func testNormalizedDropsStraySeparatorsAndEmptySubmenus() {
        let items: [CaperMenuItem] = [
            .separator, .action("Reply") {}, .separator, .separator,
            .submenu("Empty", [.separator]), .separator,
            .submenu("Mute", [.separator, .action("For 1 hour") {}, .separator]), .separator,
        ]
        XCTAssertEqual(shape(CaperMenuItem.normalized(items)), ["Reply", "—", "Mute[For 1 hour]"])
    }

    #if os(macOS)
    @MainActor func testItemsFromNSMenuKeepRolesStateAndActions() {
        _ = NSApplication.shared
        final class Target: NSObject { var fired = 0; @objc func fire() { fired += 1 } }
        let target = Target()
        let menu = NSMenu()
        let pin = NSMenuItem(title: "Pin message", action: #selector(Target.fire), keyEquivalent: ""); pin.target = target
        let delete = NSMenuItem(title: "Delete message", action: #selector(Target.fire), keyEquivalent: "")
        delete.target = target; delete.caperRole = .destructive
        let level = NSMenuItem(title: "Notifications", action: nil, keyEquivalent: "")
        let submenu = NSMenu()
        let all = NSMenuItem(title: "All messages", action: #selector(Target.fire), keyEquivalent: ""); all.target = target; all.state = .on
        submenu.addItem(all)
        level.submenu = submenu
        let hidden = NSMenuItem(title: "Hidden", action: #selector(Target.fire), keyEquivalent: ""); hidden.isHidden = true
        let disabled = NSMenuItem(title: "Reply", action: #selector(Target.fire), keyEquivalent: ""); disabled.isEnabled = false
        [pin, .separator(), hidden, level, NSMenuItem(title: "Muted until 5:00 PM", action: nil, keyEquivalent: ""),
         disabled, .separator(), delete, .separator()].forEach(menu.addItem)

        let items = CaperMenuItem.items(from: menu)
        XCTAssertEqual(shape(items), ["Pin message", "—", "Notifications[All messages]", "(Muted until 5:00 PM)", "Reply", "—", "Delete message"])
        XCTAssertEqual(items.map(\.isDestructive), [false, false, false, false, false, false, true])
        XCTAssertFalse(items[4].isEnabled)
        XCTAssertTrue(items[2].children?.first?.isSelected == true)
        guard case let .action(perform) = items[6].kind else { return XCTFail("Delete is an action") }
        perform()
        XCTAssertEqual(target.fired, 1)
    }
    #endif
}
