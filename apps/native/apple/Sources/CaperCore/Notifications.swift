import Foundation

/// What notifies you: the account level, or a space or channel override.
/// The server may add values later; an unknown one reads as `mentions`.
public enum NotificationLevel: String, Codable, CaseIterable, Identifiable, Sendable {
    case all, mentions, nothing

    public var id: String { rawValue }

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = NotificationLevel(rawValue: raw) ?? .mentions
    }

    /// "Notify me about" in account settings.
    public var accountTitle: String {
        switch self {
        case .all: return "All messages"
        case .mentions: return "Only @mentions and DMs"
        case .nothing: return "Nothing"
        }
    }

    /// A choice in a space or channel menu.
    public var overrideTitle: String {
        switch self {
        case .all: return "All messages"
        case .mentions: return "Only @mentions"
        case .nothing: return "Nothing"
        }
    }

    /// The menu's inherit choice names what it inherits: "Default (Only @mentions)".
    public static func defaultTitle(inheriting level: NotificationLevel) -> String {
        "Default (\(level.overrideTitle))"
    }
}

/// `mobile`: whether phone push waits while you are active on another sign-in.
public enum MobilePushPolicy: String, Codable, CaseIterable, Identifiable, Sendable {
    case whenInactive, always

    public var id: String { rawValue }

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = MobilePushPolicy(rawValue: raw) ?? .whenInactive
    }

    public var title: String {
        switch self {
        case .whenInactive: return "When I'm not active elsewhere"
        case .always: return "Always"
        }
    }
}

/// A mute's end: a time, or `"forever"` (until you turn it back on).
public enum MuteUntil: Hashable, Sendable {
    case forever
    case date(Date)

    /// `"forever"` or an RFC 3339 timestamp, with or without fractional seconds.
    public init?(serverValue: String) {
        if serverValue == "forever" { self = .forever; return }
        let formatter = ISO8601DateFormatter()
        if let date = formatter.date(from: serverValue) { self = .date(date); return }
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let date = formatter.date(from: serverValue) else { return nil }
        self = .date(date)
    }

    /// Whole-second UTC, as the API stores it.
    public var serverValue: String {
        switch self {
        case .forever: return "forever"
        case let .date(date): return ISO8601DateFormatter().string(from: date)
        }
    }

    public func isActive(at now: Date) -> Bool {
        switch self {
        case .forever: return true
        case let .date(date): return date > now
        }
    }
}

/// The mute choices every menu offers, in order.
public enum MutePreset: CaseIterable, Identifiable, Sendable {
    case fifteenMinutes, oneHour, eightHours, twentyFourHours, forever

    public var id: Self { self }

    public var title: String {
        switch self {
        case .fifteenMinutes: return "For 15 minutes"
        case .oneHour: return "For 1 hour"
        case .eightHours: return "For 8 hours"
        case .twentyFourHours: return "For 24 hours"
        case .forever: return "Until I turn it back on"
        }
    }

    private var seconds: TimeInterval? {
        switch self {
        case .fifteenMinutes: return 900
        case .oneHour: return 3_600
        case .eightHours: return 28_800
        case .twentyFourHours: return 86_400
        case .forever: return nil
        }
    }

    public func until(from now: Date) -> MuteUntil {
        guard let seconds else { return .forever }
        return .date(now.addingTimeInterval(seconds))
    }
}

/// What an override applies to.
public enum NotificationScope: Hashable, Sendable {
    case space(String)
    case channel(spaceID: String, channelID: String)
    case direct(String)

    /// The word in "Mute space", "Mute channel" and "Mute conversation".
    public var noun: String {
        switch self {
        case .space: return "space"
        case .channel: return "channel"
        case .direct: return "conversation"
        }
    }
}

/// One space, channel or DM override. A null level inherits; a null mute is unmuted.
public struct NotificationOverride: Decodable, Hashable, Sendable {
    public let scope: NotificationScope
    public var level: NotificationLevel?
    public var mutedUntil: MuteUntil?

    /// Neither a level nor a mute: the server omits these.
    public var isEmpty: Bool { level == nil && mutedUntil == nil }

    public init(scope: NotificationScope, level: NotificationLevel? = nil, mutedUntil: MuteUntil? = nil) {
        self.scope = scope; self.level = level; self.mutedUntil = mutedUntil
    }

    private enum CodingKeys: String, CodingKey { case spaceId, channelId, conversationId, level, mutedUntil }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let spaceID = try values.decodeIfPresent(String.self, forKey: .spaceId)
        let channelID = try values.decodeIfPresent(String.self, forKey: .channelId)
        let conversationID = try values.decodeIfPresent(String.self, forKey: .conversationId)
        if let conversationID {
            scope = .direct(conversationID)
        } else if let spaceID, let channelID {
            scope = .channel(spaceID: spaceID, channelID: channelID)
        } else if let spaceID {
            scope = .space(spaceID)
        } else {
            throw DecodingError.dataCorrupted(DecodingError.Context(codingPath: decoder.codingPath,
                                                                   debugDescription: "A notification override needs a scope."))
        }
        let rawLevel: NotificationLevel? = try? values.decodeIfPresent(NotificationLevel.self, forKey: .level)
        level = rawLevel
        let rawMute: String? = try? values.decodeIfPresent(String.self, forKey: .mutedUntil)
        mutedUntil = rawMute.flatMap { MuteUntil(serverValue: $0) }
    }
}

/// A `PUT .../notifications` body. A field left out stays as it is; `null` resets it.
public struct NotificationOverrideChange: Encodable, Equatable, Sendable {
    public var level: NotificationLevel??
    public var mutedUntil: MuteUntil??

    public init(level: NotificationLevel?? = .none, mutedUntil: MuteUntil?? = .none) {
        self.level = level; self.mutedUntil = mutedUntil
    }

    public static func level(_ level: NotificationLevel?) -> NotificationOverrideChange {
        NotificationOverrideChange(level: .some(level))
    }

    public static func mute(_ until: MuteUntil?) -> NotificationOverrideChange {
        NotificationOverrideChange(mutedUntil: .some(until))
    }

    private enum CodingKeys: String, CodingKey { case level, mutedUntil }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        if let level {
            if let value = level { try container.encode(value.rawValue, forKey: .level) }
            else { try container.encodeNil(forKey: .level) }
        }
        if let mutedUntil {
            if let value = mutedUntil { try container.encode(value.serverValue, forKey: .mutedUntil) }
            else { try container.encodeNil(forKey: .mutedUntil) }
        }
    }
}

/// A `PUT /api/notifications/settings` body; nil fields are left out.
public struct NotificationAccountChange: Encodable, Equatable, Sendable {
    public var level: NotificationLevel?
    public var mobile: MobilePushPolicy?

    public init(level: NotificationLevel? = nil, mobile: MobilePushPolicy? = nil) {
        self.level = level; self.mobile = mobile
    }
}

/// `GET /api/notifications/settings`.
public struct NotificationSettings: Decodable, Equatable, Sendable {
    public var level: NotificationLevel
    public var mobile: MobilePushPolicy
    public var overrides: [NotificationOverride]

    public init(level: NotificationLevel = .all, mobile: MobilePushPolicy = .whenInactive, overrides: [NotificationOverride] = []) {
        self.level = level; self.mobile = mobile; self.overrides = overrides
    }

    private enum CodingKeys: String, CodingKey { case level, mobile, overrides }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let decodedLevel: NotificationLevel? = try? values.decodeIfPresent(NotificationLevel.self, forKey: .level)
        level = decodedLevel ?? .all
        let decodedMobile: MobilePushPolicy? = try? values.decodeIfPresent(MobilePushPolicy.self, forKey: .mobile)
        mobile = decodedMobile ?? .whenInactive
        let entries: [LossyOverride] = try values.decodeIfPresent([LossyOverride].self, forKey: .overrides) ?? []
        overrides = entries.compactMap(\.value)
    }

    /// An override with a shape this client does not know is skipped, not fatal.
    private struct LossyOverride: Decodable {
        let value: NotificationOverride?
        init(from decoder: Decoder) throws { value = try? NotificationOverride(from: decoder) }
    }

    // MARK: Reading

    public func override(for scope: NotificationScope) -> NotificationOverride? {
        overrides.first { $0.scope == scope }
    }

    /// The scope's own level; nil inherits.
    public func overrideLevel(for scope: NotificationScope) -> NotificationLevel? {
        override(for: scope)?.level
    }

    /// The level a null override inherits: a channel takes its space's, a space the account's.
    public func inheritedLevel(for scope: NotificationScope) -> NotificationLevel {
        guard case let .channel(spaceID, _) = scope else { return level }
        return overrideLevel(for: .space(spaceID)) ?? level
    }

    /// The scope's own mute while it lasts. Expired mutes read as nil.
    public func ownMute(for scope: NotificationScope, now: Date) -> MuteUntil? {
        guard let mute = override(for: scope)?.mutedUntil, mute.isActive(at: now) else { return nil }
        return mute
    }

    /// A channel whose space is muted ("Muted with the space").
    public func isMutedWithSpace(_ scope: NotificationScope, now: Date) -> Bool {
        guard case let .channel(spaceID, _) = scope else { return false }
        return ownMute(for: .space(spaceID), now: now) != nil
    }

    /// Muted itself or, for a channel, through its space. Rows dim and show a bell-slash.
    public func isMuted(_ scope: NotificationScope, now: Date) -> Bool {
        ownMute(for: scope, now: now) != nil || isMutedWithSpace(scope, now: now)
    }

    /// A DM's "Turn off notifications" (its level `nothing`).
    public func notificationsOff(_ scope: NotificationScope) -> Bool {
        overrideLevel(for: scope) == .nothing
    }

    /// When the next timed mute ends, so the sidebar can redraw then.
    public func nextMuteExpiry(after now: Date) -> Date? {
        var next: Date?
        for entry in overrides {
            guard case let .date(date)? = entry.mutedUntil, date > now else { continue }
            if let earliest = next, earliest <= date { continue }
            next = date
        }
        return next
    }

    // MARK: Changing

    /// Replaces the scope's override (or clears it when `override` is nil or empty).
    public func replacing(_ override: NotificationOverride?, for scope: NotificationScope) -> NotificationSettings {
        var copy = self
        copy.overrides.removeAll { $0.scope == scope }
        if let override, override.scope == scope, !override.isEmpty { copy.overrides.append(override) }
        return copy
    }

    /// The optimistic result of an override change.
    public func applying(_ change: NotificationOverrideChange, to scope: NotificationScope) -> NotificationSettings {
        var next = override(for: scope) ?? NotificationOverride(scope: scope)
        if let level = change.level { next.level = level }
        if let mute = change.mutedUntil { next.mutedUntil = mute }
        return replacing(next, for: scope)
    }

    /// The optimistic result of an account change.
    public func applying(_ change: NotificationAccountChange) -> NotificationSettings {
        var copy = self
        if let level = change.level { copy.level = level }
        if let mobile = change.mobile { copy.mobile = mobile }
        return copy
    }
}

/// Menu and settings copy shared by iOS and macOS.
public enum NotificationLabels {
    public static let notifications = "Notifications"
    public static let notifyMeAbout = "Notify me about"
    public static let sendToThisPhone = "Send to this phone"
    public static let mutedWithSpace = "Muted with the space"
    public static let saveFailed = "Couldn't save your notification settings. Try again."
    public static let loadFailed = "Couldn't load your notification settings."

    /// "Mute space" / "Unmute channel" / "Mute conversation".
    public static func muteTitle(for scope: NotificationScope, muted: Bool) -> String {
        "\(muted ? "Unmute" : "Mute") \(scope.noun)"
    }

    /// A DM's on/off item.
    public static func directNotificationsTitle(off: Bool) -> String {
        off ? "Turn on notifications" : "Turn off notifications"
    }

    /// "Muted" with no end, else "Muted until 5:00 PM" in local time, with the
    /// date when it isn't today and the year when it isn't this year.
    public static func mutedStatus(_ mute: MuteUntil, now: Date = Date(), calendar: Calendar = .current,
                                   locale: Locale = .current) -> String {
        guard case let .date(date) = mute else { return "Muted" }
        let time = DateFormatter()
        time.locale = locale
        time.timeZone = calendar.timeZone
        time.dateStyle = .none
        time.timeStyle = .short
        let clock = time.string(from: date)
        if calendar.isDate(date, inSameDayAs: now) { return "Muted until \(clock)" }
        let sameYear = calendar.component(.year, from: date) == calendar.component(.year, from: now)
        let day = DateFormatter()
        day.locale = locale
        day.timeZone = calendar.timeZone
        day.setLocalizedDateFormatFromTemplate(sameYear ? "MMMd" : "yMMMd")
        return "Muted until \(day.string(from: date)), \(clock)"
    }
}

/// Where a tapped push leads, from the APNs payload's custom keys.
public enum NotificationRoute: Equatable, Sendable {
    case channel(spaceID: String, channelID: String)
    case direct(conversationID: String)

    /// DMs carry `conversationId`; channels carry `spaceId` and `channelId`.
    public init?(userInfo: [AnyHashable: Any]) {
        func value(_ key: String) -> String? {
            guard let string = userInfo[key] as? String, !string.isEmpty else { return nil }
            return string
        }
        if let conversationID = value("conversationId") {
            self = .direct(conversationID: conversationID)
        } else if let spaceID = value("spaceId"), let channelID = value("channelId") {
            self = .channel(spaceID: spaceID, channelID: channelID)
        } else {
            return nil
        }
    }

    /// Whether this is the conversation on screen, so a foreground banner is redundant.
    public func isOpen(spaceID: String?, channelID: String?, directMessageID: String?) -> Bool {
        switch self {
        case let .direct(conversationID):
            return directMessageID == conversationID
        case let .channel(routeSpaceID, routeChannelID):
            return directMessageID == nil && spaceID == routeSpaceID && channelID == routeChannelID
        }
    }
}
