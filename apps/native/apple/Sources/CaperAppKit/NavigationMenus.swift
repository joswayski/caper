import AppKit
import CaperCore

/// NSMenu owns its action items until it closes. No parallel notification state:
/// labels, inheritance, persistence and pending guards come from CaperCore.
final class NavigationMenuItem: NSMenuItem, NSMenuItemValidation {
    private let perform: () -> Void
    private let available: Bool
    init(_ title: String, enabled: Bool = true, selected: Bool = false, action: @escaping () -> Void) {
        perform = action
        available = enabled
        super.init(title: title, action: #selector(invoke), keyEquivalent: "")
        target = self
        isEnabled = enabled
        state = selected ? .on : .off
        // The visible title alone can leave the custom action's AX label empty.
        setAccessibilityLabel(title)
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func invoke() { perform() }
    // NSTextView's own menu enables items automatically. Keep permissions
    // when these action items are appended to that native context menu.
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool { available }
}

@MainActor enum NavigationMenus {
    static func menu(_ items: [NSMenuItem]) -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false
        items.forEach { menu.addItem($0) }
        return menu
    }

    static func submenu(_ title: String, _ items: [NSMenuItem]) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.submenu = menu(items)
        return item
    }

    static func space(_ model: AppModel, id: String) -> [NSMenuItem] {
        let scope = NotificationScope.space(id)
        let choices: [(Bool?, String)] = [(nil, "Default (account)"), (true, "On"), (false, "Off")]
        let voice = choices.map { value, title in
            NavigationMenuItem(title, enabled: !saving(model, scope),
                selected: model.notificationSettings?.override(for: scope)?.voiceStarted == value) {
                Task { await model.setVoiceStartedOverride(value, spaceID: id) }
            }
        }
        return [level(model, scope), submenu("When voice starts", voice)] + mute(model, scope)
    }

    static func channel(_ model: AppModel, channel: Channel) -> [NSMenuItem] {
        let scope = NotificationScope.channel(spaceID: channel.spaceId, channelID: channel.id)
        var items = [level(model, scope)]
        if model.notificationsMutedWithSpace(scope) { items.append(NavigationMenuItem(NotificationLabels.mutedWithSpace, enabled: false) {}) }
        return items + mute(model, scope)
    }

    static func direct(_ model: AppModel, id: String) -> [NSMenuItem] {
        let scope = NotificationScope.direct(id)
        let off = model.notificationSettings?.notificationsOff(scope) ?? false
        return [NavigationMenuItem(NotificationLabels.directNotificationsTitle(off: off), enabled: !saving(model, scope)) {
            Task { await model.setNotificationLevel(off ? nil : .nothing, for: scope) }
        }] + mute(model, scope)
    }

    private static func saving(_ model: AppModel, _ scope: NotificationScope) -> Bool {
        model.notificationSettings == nil || model.notificationScopesSaving.contains(scope)
    }

    private static func level(_ model: AppModel, _ scope: NotificationScope) -> NSMenuItem {
        let inherited = model.notificationSettings?.inheritedLevel(for: scope) ?? .all
        let current = model.notificationSettings?.overrideLevel(for: scope)
        let choices: [NotificationLevel?] = [nil, .all, .mentions, .nothing]
        return submenu(NotificationLabels.notifications, choices.map { value in
            NavigationMenuItem(value?.overrideTitle ?? NotificationLevel.defaultTitle(inheriting: inherited),
                enabled: !saving(model, scope), selected: current == value) {
                Task { await model.setNotificationLevel(value, for: scope) }
            }
        })
    }

    private static func mute(_ model: AppModel, _ scope: NotificationScope) -> [NSMenuItem] {
        if let mute = model.notificationMute(scope) {
            return [NavigationMenuItem(NotificationLabels.mutedStatus(mute), enabled: false) {},
                NavigationMenuItem(NotificationLabels.muteTitle(for: scope, muted: true), enabled: !saving(model, scope)) {
                    Task { await model.setMute(nil, for: scope) }
                }]
        }
        return [submenu(NotificationLabels.muteTitle(for: scope, muted: false), MutePreset.allCases.map { preset in
            NavigationMenuItem(preset.title, enabled: !saving(model, scope)) {
                Task { await model.setMute(preset.until(from: Date()), for: scope) }
            }
        })]
    }
}
