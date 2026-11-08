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
    var location: String { [spaceName, channelName].compactMap { $0 }.joined(separator: " · ") }
    var route: NotificationRoute? {
        if kind == "space.invitation", let spaceId { return .invitation(spaceID: spaceId, channelID: nil) }
        if kind == "channel.invitation", let spaceId, let channelId { return .invitation(spaceID: spaceId, channelID: channelId) }
        if let conversationId { return .direct(conversationID: conversationId) }
        if let spaceId, let channelId { return .channel(spaceID: spaceId, channelID: channelId) }
        return nil
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

struct ActivityInboxView: View {
    @Bindable var model: AppModel
    var close: (() -> Void)? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var reactionItem: ActivityItem?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    #if os(macOS)
    private let actionSize: CGFloat = 32
    #else
    private let actionSize: CGFloat = 44
    #endif
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
                    #if os(macOS)
                    .controlSize(.mini)
                    #else
                    .scaleEffect(0.75).frame(width: 40, height: 44)
                    #endif
                    .onChange(of: model.activityUnreadOnly) { _, _ in Task { await model.loadActivity(reset: true) } }
                    .accessibilityIdentifier("activity-unread-only")
                Text("Unread only ") + Text("(\(model.activityUnread))").foregroundColor(CaperTheme.muted)
                Spacer()
            }
            if let error = model.activityError {
                HStack {
                    Text(error).foregroundStyle(CaperTheme.terracotta)
                    Button("Retry") { Task { await model.loadActivity(reset: true, poll: true) } }
                        .disabled(model.activityLoading)
                }
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    if model.activityLoading && model.activityVisibleItems.isEmpty { ProgressView("Loading notifications…") }
                    if !model.activityLoading && model.activityVisibleItems.isEmpty && model.activityError == nil {
                        Text("No notifications match these filters.").foregroundStyle(CaperTheme.muted).padding(.vertical, 24)
                    }
                    ForEach(model.activityVisibleItems) { item in
                        VStack(alignment: .leading, spacing: 6) {
                            ZStack(alignment: .topTrailing) {
                                Button {
                                    Task { if await model.openActivity(item) { closeInbox() } }
                                } label: {
                                    VStack(alignment: .leading, spacing: 5) {
                                        Text("\(item.unread ? "● " : "")\(item.actorName)")
                                            .font(CaperTheme.font(13, weight: item.unread ? .bold : .medium))
                                            .padding(.trailing, 110).frame(minHeight: actionSize, alignment: .leading)
                                        if let description = item.description { Text(description).foregroundStyle(CaperTheme.muted) }
                                        Text(item.body).lineLimit(3)
                                        Text(item.location).foregroundStyle(CaperTheme.muted)
                                    }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                                }.buttonStyle(.plain).disabled(model.activityOpening == item.id)
                                    .accessibilityIdentifier("activity-open-\(item.id)")
                                HStack(spacing: 6) {
                                    Text(FeedbackInbox.relativeTime(item.createdAt)).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                                    if item.unread {
                                        Button { Task { _ = await model.readActivity(item) } } label: {
                                            Image(systemName: "checkmark").frame(width: actionSize, height: actionSize).contentShape(Rectangle())
                                        }.buttonStyle(.plain)
                                            .accessibilityLabel("Mark notification from \(item.actorName) read")
                                            .help("Mark read without opening")
                                            .accessibilityIdentifier("activity-read-\(item.id)")
                                    }
                                }.frame(minHeight: actionSize)
                            }
                            if model.canReplyToActivity(item) {
                                HStack(spacing: 16) {
                                    Button {
                                        if model.activityExpanded[item.id] != nil { model.activityExpanded[item.id] = nil }
                                        else { model.activityExpanded[item.id] = item }
                                    } label: {
                                        Text(model.activityExpanded[item.id] == nil ? "Reply" : "Close reply")
                                            .frame(minWidth: 44, minHeight: actionSize).contentShape(Rectangle())
                                    }.accessibilityIdentifier("activity-reply-\(item.id)")
                                    Button { reactionItem = item } label: {
                                        Text("React").frame(minWidth: 44, minHeight: actionSize).contentShape(Rectangle())
                                    }.accessibilityIdentifier("activity-react-\(item.id)")
                                }.buttonStyle(.plain).foregroundStyle(CaperTheme.muted)
                            }
                            if model.activityExpanded[item.id] != nil {
                                VStack(alignment: .leading, spacing: 8) {
                                    Text(item.replyLabel).foregroundStyle(CaperTheme.muted)
                                    MentionCardView(model: model, activity: item)
                                }.modifier(Entrance.anchored)
                            }
                            Divider()
                        }.accessibilityElement(children: .contain)
                            .accessibilityIdentifier("activity-row-\(item.id)")
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
            #if os(macOS)
            // Native workspace dialogs supply the available size; the shipping
            // sheet retains its fixed dimensions and environment dismissal.
            .frame(idealWidth: 680, idealHeight: 640)
            .frame(width: close == nil ? 680 : nil, height: close == nil ? 640 : nil)
            #else
            .frame(minWidth: 300, idealWidth: 560, minHeight: 420, idealHeight: 640)
            #endif
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("activity-inbox-dialog")
            .task {
                await model.loadActivity(reset: true)
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .seconds(15)) } catch { return }
                    if !model.activityLoading { await model.loadActivity(reset: true, poll: true) }
                }
            }
    }

    private func closeInbox() { if let close { close() } else { dismiss() } }
}
