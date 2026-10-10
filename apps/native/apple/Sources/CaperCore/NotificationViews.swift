import SwiftUI

// Menu items for notification levels and mutes. Each piece is its own small
// view so the type checker never sees one large menu expression.

/// "Notifications" ▸ Default (…) / All messages / Only @mentions / Nothing, for a space or channel.
struct NotificationLevelMenu: View {
    @Bindable var model: AppModel
    let scope: NotificationScope

    var body: some View {
        Menu(NotificationLabels.notifications) {
            choice(nil)
            choice(.all)
            choice(.mentions)
            choice(.nothing)
        }
        .disabled(model.notificationSettings == nil)
        .onAppear { model.refreshNotificationSettingsIfStale() }
    }

    private var inherited: NotificationLevel {
        model.notificationSettings?.inheritedLevel(for: scope) ?? .all
    }

    private func title(_ level: NotificationLevel?) -> String {
        guard let level else { return NotificationLevel.defaultTitle(inheriting: inherited) }
        return level.overrideTitle
    }

    private func choice(_ level: NotificationLevel?) -> some View {
        let selected: Bool = model.notificationSettings?.overrideLevel(for: scope) == level
        let binding = Binding<Bool>(
            get: { selected },
            set: { on in if on { Task { await model.setNotificationLevel(level, for: scope) } } }
        )
        return Toggle(title(level), isOn: binding)
    }
}

/// "Mute …" ▸ presets, or "Unmute …" with "Muted until 5:00 PM" while muted.
struct NotificationMuteMenu: View {
    @Bindable var model: AppModel
    let scope: NotificationScope

    var body: some View {
        if let mute = model.notificationMute(scope) {
            Button(NotificationLabels.mutedStatus(mute)) {}.disabled(true)
            Button(NotificationLabels.muteTitle(for: scope, muted: true)) { save(nil) }
        } else {
            Menu(NotificationLabels.muteTitle(for: scope, muted: false)) {
                preset(.fifteenMinutes)
                preset(.oneHour)
                preset(.eightHours)
                preset(.twentyFourHours)
                preset(.forever)
            }
            .disabled(model.notificationSettings == nil)
        }
    }

    private func preset(_ preset: MutePreset) -> some View {
        Button(preset.title) { save(preset.until(from: Date())) }
    }

    private func save(_ until: MuteUntil?) {
        Task { await model.setMute(until, for: scope) }
    }
}

/// The space menu's notification items.
struct SpaceNotificationItems: View {
    @Bindable var model: AppModel
    let spaceID: String

    var body: some View {
        NotificationLevelMenu(model: model, scope: .space(spaceID))
        NotificationMuteMenu(model: model, scope: .space(spaceID))
    }
}

/// A channel's options-menu items. A channel in a muted space says so and
/// keeps its own mute choices.
struct ChannelNotificationItems: View {
    @Bindable var model: AppModel
    let channel: Channel

    private var scope: NotificationScope { .channel(spaceID: channel.spaceId, channelID: channel.id) }

    var body: some View {
        NotificationLevelMenu(model: model, scope: scope)
        if model.notificationsMutedWithSpace(scope) {
            Button(NotificationLabels.mutedWithSpace) {}.disabled(true)
        }
        NotificationMuteMenu(model: model, scope: scope)
    }
}

/// A DM row's options: notifications on or off, then mute.
struct DirectMessageNotificationItems: View {
    @Bindable var model: AppModel
    let conversationID: String

    private var scope: NotificationScope { .direct(conversationID) }
    private var off: Bool { model.notificationSettings?.notificationsOff(scope) ?? false }

    var body: some View {
        Button(NotificationLabels.directNotificationsTitle(off: off)) {
            let level: NotificationLevel? = off ? nil : .nothing
            Task { await model.setNotificationLevel(level, for: scope) }
        }
        .disabled(model.notificationSettings == nil)
        .onAppear { model.refreshNotificationSettingsIfStale() }
        NotificationMuteMenu(model: model, scope: scope)
    }
}

/// The bell-slash on muted spaces, channels and DMs.
struct MutedBell: View {
    var size: CGFloat = 12

    var body: some View {
        Image(systemName: "bell.slash")
            .font(.system(size: size, weight: .medium))
            .foregroundStyle(CaperTheme.muted)
            .accessibilityHidden(true)
    }
}

/// A failed notification change, under the sidebar header until dismissed.
struct NotificationErrorRow: View {
    @Bindable var model: AppModel

    var body: some View {
        if let message = model.notificationError {
            HStack(alignment: .top, spacing: 8) {
                Text(message)
                    .font(CaperTheme.font(11))
                    .foregroundStyle(CaperTheme.terracottaBright)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button("Dismiss") { model.notificationError = nil }
                    .buttonStyle(.plain)
                    .font(CaperTheme.font(11, weight: .bold))
                    .foregroundStyle(CaperTheme.muted)
                    .modifier(ControlPointer())
            }
            .padding(.horizontal, 16).padding(.vertical, 8)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("notification-error")
        }
    }
}
