import Foundation

/// One entry of a Caper menu: the app's own popup and context menu, drawn the
/// same on every client (web's `.space-actions` menu) instead of the system's.
/// AppKit draws it with `CaperMenu` (AppKitCaperMenu.swift). Submenus expand
/// in place under their row, as on the website, rather than cascading.
public struct CaperMenuItem {
    public enum Kind {
        case action(() -> Void)
        case submenu([CaperMenuItem])
        /// Muted information, such as "Muted until 5:00 PM".
        case note
        case separator
    }

    public var title: String
    public var kind: Kind
    public var isEnabled: Bool
    /// The current choice: a terracotta check before the title.
    public var isSelected: Bool
    /// Delete, Block, Leave: error-coloured.
    public var isDestructive: Bool
    /// The accessibility identifier; the title when nil.
    public var identifier: String?

    public init(title: String, kind: Kind, isEnabled: Bool = true, isSelected: Bool = false,
                isDestructive: Bool = false, identifier: String? = nil) {
        self.title = title
        self.kind = kind
        self.isEnabled = isEnabled
        self.isSelected = isSelected
        self.isDestructive = isDestructive
        self.identifier = identifier
    }

    public static func action(_ title: String, enabled: Bool = true, selected: Bool = false, destructive: Bool = false,
                              identifier: String? = nil, _ perform: @escaping () -> Void) -> CaperMenuItem {
        CaperMenuItem(title: title, kind: .action(perform), isEnabled: enabled, isSelected: selected,
                      isDestructive: destructive, identifier: identifier)
    }

    public static func submenu(_ title: String, enabled: Bool = true, _ items: [CaperMenuItem]) -> CaperMenuItem {
        CaperMenuItem(title: title, kind: .submenu(items), isEnabled: enabled)
    }

    public static func note(_ title: String) -> CaperMenuItem {
        CaperMenuItem(title: title, kind: .note, isEnabled: false)
    }

    public static var separator: CaperMenuItem { CaperMenuItem(title: "", kind: .separator) }

    var isSeparator: Bool { if case .separator = kind { true } else { false } }
    var children: [CaperMenuItem]? { if case let .submenu(items) = kind { items } else { nil } }
    /// Rows the keyboard and VoiceOver can act on.
    var isInteractive: Bool {
        switch kind {
        case .action, .submenu: isEnabled
        case .note, .separator: false
        }
    }

    /// Drops empty submenus and leading, trailing and repeated separators, so
    /// builders can append groups without tracking what came before.
    public static func normalized(_ items: [CaperMenuItem]) -> [CaperMenuItem] {
        var result: [CaperMenuItem] = []
        for var item in items {
            if let children = item.children {
                let kept = normalized(children)
                if kept.isEmpty { continue }
                item.kind = .submenu(kept)
            }
            if item.isSeparator, result.last?.isSeparator ?? true { continue }
            result.append(item)
        }
        if result.last?.isSeparator == true { result.removeLast() }
        return result
    }
}

/// The menu's measurements (web: 5px padding,
/// 8px corners, 6px item corners, 8px item inset, 9px icon gap).
enum CaperMenuMetrics {
    static let padding: CGFloat = 5
    static let cornerRadius: CGFloat = 8
    static let itemRadius: CGFloat = 6
    static let itemInset: CGFloat = 8
    static let checkWidth: CGFloat = 16
    static let gap: CGFloat = 9
    /// Expanded choices step in under their submenu's title.
    static let childIndent: CGFloat = 12
    static let separatorHeight: CGFloat = 11
    static let edgeMargin: CGFloat = 8
}
