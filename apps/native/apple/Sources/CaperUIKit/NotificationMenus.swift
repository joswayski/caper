import CaperCore
import UIKit

/// The notification menus SwiftUI's NotificationViews draw: level choices,
/// "When voice starts" for a space, and mute presets or Unmute.
@MainActor enum NotificationMenus {
    static func space(_ model: AppModel, spaceID: String) -> [UIMenuElement] {
        let scope = NotificationScope.space(spaceID)
        let voice = UIMenu(title: "When voice starts", options: saving(model, scope) ? .init() : [], children: [
            (nil, "Default (account)"), (true, "On"), (false, "Off"),
        ].map { (value: Bool?, title: String) in
            let current = model.notificationSettings?.override(for: scope)?.voiceStarted == value
            return UIAction(title: title, attributes: saving(model, scope) ? .disabled : [], state: current ? .on : .off) { _ in
                Task { await model.setVoiceStartedOverride(value, spaceID: spaceID) }
            }
        })
        return [level(model, scope), voice, mute(model, scope)]
    }

    static func channel(_ model: AppModel, channel: Channel) -> [UIMenuElement] {
        let scope = NotificationScope.channel(spaceID: channel.spaceId, channelID: channel.id)
        var items: [UIMenuElement] = [level(model, scope)]
        if model.notificationsMutedWithSpace(scope) {
            items.append(UIAction(title: NotificationLabels.mutedWithSpace, attributes: .disabled) { _ in })
        }
        items.append(mute(model, scope))
        return items
    }

    static func direct(_ model: AppModel, conversationID: String) -> [UIMenuElement] {
        let scope = NotificationScope.direct(conversationID)
        let off = model.notificationSettings?.notificationsOff(scope) ?? false
        return [UIAction(title: NotificationLabels.directNotificationsTitle(off: off),
                         attributes: saving(model, scope) ? .disabled : []) { _ in
            Task { await model.setNotificationLevel(off ? nil : .nothing, for: scope) }
        }, mute(model, scope)]
    }

    private static func saving(_ model: AppModel, _ scope: NotificationScope) -> Bool {
        model.notificationSettings == nil || model.notificationScopesSaving.contains(scope)
    }

    private static func level(_ model: AppModel, _ scope: NotificationScope) -> UIMenuElement {
        let inherited = model.notificationSettings?.inheritedLevel(for: scope) ?? .all
        let current = model.notificationSettings?.overrideLevel(for: scope)
        let choices: [NotificationLevel?] = [nil, .all, .mentions, .nothing]
        return UIMenu(title: NotificationLabels.notifications, children: choices.map { level in
            UIAction(title: level?.overrideTitle ?? NotificationLevel.defaultTitle(inheriting: inherited),
                     attributes: saving(model, scope) ? .disabled : [], state: current == level ? .on : .off) { _ in
                Task { await model.setNotificationLevel(level, for: scope) }
            }
        })
    }

    private static func mute(_ model: AppModel, _ scope: NotificationScope) -> UIMenuElement {
        if let current = model.notificationMute(scope) {
            return UIMenu(options: .displayInline, children: [
                UIAction(title: NotificationLabels.mutedStatus(current), attributes: .disabled) { _ in },
                UIAction(title: NotificationLabels.muteTitle(for: scope, muted: true),
                         attributes: model.notificationScopesSaving.contains(scope) ? .disabled : []) { _ in
                    Task { await model.setMute(nil, for: scope) }
                },
            ])
        }
        return UIMenu(title: NotificationLabels.muteTitle(for: scope, muted: false), children: MutePreset.allCases.map { preset in
            UIAction(title: preset.title, attributes: saving(model, scope) ? .disabled : []) { _ in
                Task { await model.setMute(preset.until(from: Date()), for: scope) }
            }
        })
    }
}
