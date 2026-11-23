import Foundation
import SwiftUI

struct ActivityItem: Decodable, Identifiable, Sendable {
    let id: String
    let kind: String
    let actorId: String
    let actorName: String
    let avatarId: Int?
    let body: String
    let createdAt: String
    var unread: Bool
    let spaceId: String?
    let spaceName: String?
    let channelId: String?
    let channelName: String?
    let conversationId: String?
    let messageId: String?
    let threadRootId: String?
    let emoji: String?

    var canReply: Bool {
        ["message", "mention", "reply", "reaction"].contains(kind)
            && messageId?.isEmpty == false && (conversationId ?? channelId)?.isEmpty == false
    }
    // Length-delimited components avoid collisions; notification IDs remain opaque.
    var draftKey: String {
        "inbox:" + [id, conversationId ?? channelId ?? "", messageId ?? "", threadRootId ?? ""]
            .map { "\($0.utf8.count):\($0)" }.joined()
    }
    var replyLabel: String {
        threadRootId != nil ? "Reply in thread" : conversationId != nil ? "Reply in DM" : "Reply in #\(channelName ?? "channel")"
    }

    var description: String? {
        switch kind {
        case "reaction": "Reacted \(emoji ?? "") to your message"
        case "direct.request": "Wants to send you messages"
        case "space.invitation": "Invited you to a space"
        case "channel.invitation": "Invited you to a private channel"
        default: nil
        }
    }
    /// A direct message has no space; its channel name is only the internal "direct".
    var location: String {
        guard let spaceName, !spaceName.isEmpty else { return "Direct message" }
        return [spaceName, channelName].compactMap { $0 }.joined(separator: " · ")
    }
    var route: NotificationRoute? {
        if kind == "space.invitation", let spaceId { return .invitation(spaceID: spaceId, channelID: nil) }
        if kind == "channel.invitation", let spaceId, let channelId { return .invitation(spaceID: spaceId, channelID: channelId) }
        if let conversationId { return .direct(conversationID: conversationId) }
        if let spaceId, let channelId { return .channel(spaceID: spaceId, channelID: channelId) }
        return nil
    }
}

/// Neighbouring notifications about the same conversation (or reactions to the
/// same message) show as one row, newest first; invitations and requests always
/// stand alone. Every client groups by the same rule; the inbox API stays per
/// notification. `items` is never empty.
struct ActivityGroup: Identifiable, Sendable {
    let items: [ActivityItem]

    var newest: ActivityItem { items[0] }
    var oldest: ActivityItem { items[items.count - 1] }
    /// The row's stable identity: list key, accessibility identifiers, expanded state.
    var id: String { newest.id }
    var ids: [String] { items.map(\.id) }
    /// Grouped items share their unread state (it is part of the key).
    var unread: Bool { newest.unread }

    /// The newest actor first, then "A and B" or "A and N others".
    var names: String {
        var seen = Set<String>()
        let distinct = items.map(\.actorName).filter { seen.insert($0).inserted }
        switch distinct.count {
        case 1: return distinct[0]
        case 2: return "\(distinct[0]) and \(distinct[1])"
        default: return "\(distinct[0]) and \(distinct.count - 1) others"
        }
    }

    /// Shown in place of the description.
    var summary: String? {
        guard items.count > 1 else { return newest.description }
        switch newest.kind {
        case "message": return "\(items.count) messages"
        case "reply": return "\(items.count) replies"
        case "mention": return "Mentioned you \(items.count) times"
        default:
            var seen = Set<String>()
            let emojis = items.map { $0.emoji ?? "" }.filter { seen.insert($0).inserted }
            return "Reacted \(emojis.joined(separator: " ")) to your message"
        }
    }

    var readLabel: String {
        items.count > 1 ? "Mark \(items.count) notifications from \(names) read" : "Mark notification from \(names) read"
    }

    /// Nil for kinds that never group.
    static func key(_ item: ActivityItem) -> [String]? {
        switch item.kind {
        case "reaction":
            return ["reaction", item.messageId ?? "", item.unread ? "unread" : "read"]
        case "message", "reply", "mention":
            return [item.kind, item.conversationId ?? item.channelId ?? "", item.threadRootId ?? "", item.unread ? "unread" : "read"]
        default:
            return nil
        }
    }

    /// Only neighbours merge, so the list keeps its order. One read request
    /// takes at most 100 IDs, so neither does a group.
    static func grouping(_ items: [ActivityItem]) -> [ActivityGroup] {
        var groups: [[ActivityItem]] = []
        for item in items {
            if let itemKey = key(item), let last = groups.last, last.count < 100, key(last[0]) == itemKey {
                groups[groups.count - 1].append(item)
            } else {
                groups.append([item])
            }
        }
        return groups.map { ActivityGroup(items: $0) }
    }
}

struct ActivityPage: Decodable, Sendable {
    let items: [ActivityItem]
    let nextCursor: String?
    let unreadCount: Int
}
struct ActivityRead: Decodable, Sendable { let unreadCount: Int }

enum ActivityFilter: String, CaseIterable, Identifiable {
    case all, dms, mentions, threads, reactions, invitations
    var id: String { rawValue }
    var label: String { self == .dms ? "DMs" : rawValue.capitalized }
}

/// A sheet leaves the conversation, its draft, and an ongoing call intact.
#if os(iOS)
struct ActivityInboxEntry: View {
    @Bindable var model: AppModel
    var body: some View {
        Button(action: SwipeTapGuard.unlessSwiping { model.activityShown = true }) {
            HStack(spacing: 9) {
                Image(systemName: "bell").frame(width: 24)
                Text("Notifications")
                Spacer()
                if model.activityUnread > 0 { Text("\(model.activityUnread)").foregroundStyle(CaperTheme.terracotta) }
            }.font(CaperTheme.font(13, weight: .medium)).foregroundStyle(CaperTheme.text)
                .padding(.horizontal, 9).frame(minHeight: 36).contentShape(Rectangle())
        }.buttonStyle(.plain)
            .sheet(isPresented: $model.activityShown) { ActivityInboxView(model: model) }
    }
}
#endif

#if os(iOS)
struct ActivityInboxView: View {
    @Bindable var model: AppModel
    var close: (() -> Void)? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var reactionItem: ActivityItem?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private let actionSize: CGFloat = 44
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Notifications").font(CaperTheme.font(20, weight: .bold))
                Spacer()
                Button("Done", action: closeInbox).accessibilityIdentifier("activity-done")
            }
            ScrollView(.horizontal) {
                HStack {
                    ForEach(ActivityFilter.allCases) { filter in
                        Button {
                            model.activityFilter = filter
                            Task { await model.loadActivity(reset: true) }
                        } label: {
                            Text(filter.label).font(CaperTheme.font(13, weight: .medium))
                                .foregroundStyle(model.activityFilter == filter ? CaperTheme.terracottaBright : CaperTheme.muted)
                                .padding(.horizontal, 12).padding(.vertical, 8)
                                .background(model.activityFilter == filter ? CaperTheme.terracotta.opacity(0.12) : CaperTheme.raised,
                                            in: RoundedRectangle(cornerRadius: 8))
                                .overlay(RoundedRectangle(cornerRadius: 8).stroke(model.activityFilter == filter ? CaperTheme.terracotta : CaperTheme.border))
                        }.buttonStyle(.plain)
                            .accessibilityAddTraits(model.activityFilter == filter ? .isSelected : [])
                            .accessibilityIdentifier("activity-filter-\(filter.rawValue)")
                    }
                }
            }.scrollIndicators(.hidden)
            HStack {
                Toggle("Unread only", isOn: $model.activityUnreadOnly)
                    .labelsHidden().toggleStyle(.switch).tint(CaperTheme.terracotta)
                    .scaleEffect(0.75).frame(width: 40, height: 44)
                    .onChange(of: model.activityUnreadOnly) { _, _ in Task { await model.loadActivity(reset: true) } }
                    .accessibilityIdentifier("activity-unread-only")
                Text("Unread only ") + Text("(\(model.activityUnread))").foregroundColor(CaperTheme.muted)
                Spacer()
            }
            if let error = model.activityError {
                HStack {
                    Text(error).foregroundStyle(CaperTheme.terracotta)
                    Button("Retry") { model.activityError = nil; Task { await model.loadActivity(reset: true, poll: true) } }
                        .disabled(model.activityLoading)
                }
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    if model.activityLoading && model.activityVisibleItems.isEmpty { ProgressView("Loading notifications…") }
                    if !model.activityLoading && model.activityVisibleItems.isEmpty && model.activityError == nil {
                        Text("No notifications match these filters.").foregroundStyle(CaperTheme.muted).padding(.vertical, 24)
                    }
                    ForEach(model.activityVisibleGroups) { group in
                        let item = group.newest
                        let target = model.activityReplyTarget(group)
                        VStack(alignment: .leading, spacing: 6) {
                            ZStack(alignment: .topTrailing) {
                                Button {
                                    Task { if await model.openActivity(group) { closeInbox() } }
                                } label: {
                                    VStack(alignment: .leading, spacing: 5) {
                                        Text("\(group.unread ? "● " : "")\(group.names)")
                                            .font(CaperTheme.font(13, weight: group.unread ? .bold : .medium))
                                            .padding(.trailing, 110).frame(minHeight: actionSize, alignment: .leading)
                                        if let summary = group.summary { Text(summary).foregroundStyle(CaperTheme.muted) }
                                        Text(item.body).lineLimit(3)
                                        Text(item.location).foregroundStyle(CaperTheme.muted)
                                    }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                                }.buttonStyle(.plain).disabled(model.activityOpening == group.id)
                                    .accessibilityIdentifier("activity-open-\(group.id)")
                                HStack(spacing: 6) {
                                    Text(FeedbackInbox.relativeTime(item.createdAt)).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                                    if group.unread {
                                        Button { Task { _ = await model.readActivity(group) } } label: {
                                            Image(systemName: "checkmark").frame(width: actionSize, height: actionSize).contentShape(Rectangle())
                                        }.buttonStyle(.plain)
                                            .accessibilityLabel(group.readLabel)
                                            .help("Mark read without opening")
                                            .accessibilityIdentifier("activity-read-\(group.id)")
                                    }
                                }.frame(minHeight: actionSize)
                            }
                            if model.canReplyToActivity(target) {
                                HStack(spacing: 16) {
                                    Button {
                                        if model.activityExpanded[target.id] != nil { model.activityExpanded[target.id] = nil }
                                        else { model.activityExpanded[target.id] = target }
                                    } label: {
                                        Text(model.activityExpanded[target.id] == nil ? "Reply" : "Close reply")
                                            .frame(minWidth: 44, minHeight: actionSize).contentShape(Rectangle())
                                    }.accessibilityIdentifier("activity-reply-\(group.id)")
                                    Button { reactionItem = target } label: {
                                        Text("React").frame(minWidth: 44, minHeight: actionSize).contentShape(Rectangle())
                                    }.accessibilityIdentifier("activity-react-\(group.id)")
                                }.buttonStyle(.plain).foregroundStyle(CaperTheme.muted)
                            }
                            if model.activityExpanded[target.id] != nil {
                                VStack(alignment: .leading, spacing: 8) {
                                    Text(target.replyLabel).foregroundStyle(CaperTheme.muted)
                                    MentionCardView(model: model, activity: target)
                                }.modifier(Entrance.anchored)
                            }
                            Divider()
                        }.accessibilityElement(children: .contain)
                            .accessibilityIdentifier("activity-row-\(group.id)")
                    }
                    if model.activityCursor != nil {
                        Button("Load more") { Task { await model.loadActivity(reset: false) } }.disabled(model.activityLoading)
                    }
                }
                .opacity(model.activityStale ? 0.5 : 1).allowsHitTesting(!model.activityStale)
                // Fast responses swap rows without a visible dim; leaving it is instant.
                .animation(model.activityStale
                           ? (reduceMotion ? Animation.linear(duration: 0) : CaperMotion.easeOut(CaperMotion.fast)).delay(0.15) : nil,
                           value: model.activityStale)
            }
        }.font(CaperTheme.font(13)).padding(20).foregroundStyle(CaperTheme.text)
            .background(CaperTheme.surface)
            .sheet(item: $reactionItem) { item in
                let account = model.account?.id
                ReactionPicker { emoji in
                    reactionItem = nil
                    Task {
                        guard model.account?.id == account else { return }
                        await model.reactToActivity(item, emoji: emoji)
                    }
                }
            }
            .frame(minWidth: 300, idealWidth: 560, minHeight: 420, idealHeight: 640)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("activity-inbox-dialog")
            .task {
                // Rows loaded ahead show at once and refresh in place.
                await model.loadActivity(reset: true, poll: model.activityLoading || !model.activityItems.isEmpty)
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .seconds(15)) } catch { return }
                    if !model.activityLoading { await model.loadActivity(reset: true, poll: true) }
                }
            }
    }

    private func closeInbox() { if let close { close() } else { dismiss() } }
}
#endif
