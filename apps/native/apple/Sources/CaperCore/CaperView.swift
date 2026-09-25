import SwiftUI
#if os(iOS)
import MediaPlayer
#elseif os(macOS)
import AppKit
#endif

public enum CaperTheme {
    public static let blackout = Color(red: 12/255, green: 13/255, blue: 15/255)
    public static let surface = Color(red: 21/255, green: 23/255, blue: 25/255)
    public static let raised = Color(red: 28/255, green: 31/255, blue: 33/255)
    public static let sidebar = Color(red: 21/255, green: 28/255, blue: 30/255)
    public static let conversation = Color(red: 25/255, green: 33/255, blue: 35/255)
    public static let composer = Color(red: 40/255, green: 49/255, blue: 51/255)
    public static let border = Color(red: 52/255, green: 56/255, blue: 59/255)
    public static let text = Color(red: 243/255, green: 244/255, blue: 245/255)
    public static let muted = Color(red: 185/255, green: 188/255, blue: 190/255)
    public static let terracotta = Color(red: 182/255, green: 77/255, blue: 50/255)
    public static let terracottaBright = Color(red: 219/255, green: 104/255, blue: 73/255)
    public static let green = Color(red: 99/255, green: 122/255, blue: 67/255)

    public static func font(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        let name: String
        switch weight {
        case .black, .heavy: name = "Satoshi-Black"
        case .bold, .semibold: name = "Satoshi-Bold"
        case .medium: name = "Satoshi-Medium"
        default: name = "Satoshi-Regular"
        }
        return .custom(name, size: size)
    }
}

@MainActor public struct CaperRootView: View {
    @State private var model: AppModel
    @State private var announcedVoice = false
    public init(model: AppModel? = nil) { _model = State(initialValue: model ?? CaperRuntime.makeModel()) }

    public var body: some View {
        Group {
            switch model.phase {
            case .loading: LoadingView()
            case .onboarding: ProfileView(model: model)
            case .signedOut, .ready: WorkspaceView(model: model)
            }
        }
        .preferredColorScheme(.dark)
        .foregroundStyle(CaperTheme.text)
        .tint(CaperTheme.terracottaBright)
        .background(CaperTheme.blackout.ignoresSafeArea())
        .task {
            CaperFontLoader.register()
            CaperEffects.shared.preload()
            if model.phase == .loading { await model.start() }
        }
        .onChange(of: model.voice.phase) { _, new in
            if new == .idle || new == .failed || new == .joining { announcedVoice = false }
            if new == .connected, !announcedVoice {
                announcedVoice = true
                CaperEffects.shared.play(.join)
            }
        }
    }
}

private struct LoadingView: View {
    var body: some View {
        VStack(spacing: 14) {
            Wordmark()
            ProgressView().controlSize(.small)
            Text("Loading your conversations…").font(CaperTheme.font(12, weight: .medium)).foregroundStyle(CaperTheme.muted)
        }.frame(maxWidth: .infinity, maxHeight: .infinity).background(CaperTheme.blackout)
    }
}

private struct Wordmark: View {
    var body: some View {
        HStack(spacing: 0) {
            Text("caper").foregroundStyle(CaperTheme.text)
            Text(".").foregroundStyle(CaperTheme.terracottaBright)
        }.font(CaperTheme.font(22, weight: .black)).tracking(-1)
    }
}

// Asset catalogs are generated from the same pinned Lucide vectors as the web.
private struct CaperIcon: View {
    let name: String
    var size: CGFloat = 16
    var body: some View {
        Image("caper-\(name)").renderingMode(.template).resizable()
            .frame(width: size, height: size).accessibilityHidden(true)
    }
}

private enum WorkspaceSheet: Identifiable {
    case login, profile, createSpace, createChannel, manageSpace, manageChannel(Channel), leaveSpace
    var id: String {
        switch self {
        case .login: "login"
        case .profile: "profile"
        case .createSpace: "create-space"
        case .createChannel: "create-channel"
        case .manageSpace: "manage-space"
        case .manageChannel(let channel): "manage-\(channel.id)"
        case .leaveSpace: "leave-space"
        }
    }
}

private struct WorkspaceView: View {
    @Bindable var model: AppModel
    @State private var sheet: WorkspaceSheet?
    @AppStorage("caper.channelSidebarWidth") private var sidebarWidth = 280.0
    @State private var sidebarDragStart: Double?
    @FocusState private var sidebarFocused: Bool
    @State private var membersPreference: Bool?
    private let parityFixture: String?

    init(model: AppModel) {
        self.model = model
        let environment = ProcessInfo.processInfo.environment
        parityFixture = environment["CAPER_TEST_MODE"] == "parity" ? environment["CAPER_UI_FIXTURE"] : nil
        _sheet = State(initialValue: parityFixture == "login" ? .login : nil)
    }

    var body: some View {
        GeometryReader { geometry in
            let narrow = geometry.size.width <= 760
            let membersVisible = membersPreference ?? !narrow
            VStack(spacing: 0) {
                if let error = model.navigationError {
                    HStack {
                        Text(error).font(CaperTheme.font(12)).foregroundStyle(.red)
                        Spacer()
                        Button("Retry opening conversation") { Task { await model.retryNavigation() } }
                            .disabled(model.openingSpaceID != nil)
                    }.padding(12).background(CaperTheme.surface)
                }
                Group {
                    if narrow && !model.navigationOpen {
                        ZStack(alignment: .trailing) {
                            ConversationStage(model: model, narrow: true, browse: { model.navigationOpen = true }, membersVisible: membersVisible) {
                                membersPreference = !membersVisible
                            }
                            if membersVisible {
                                MemberPresenceView(model: model)
                                    .frame(width: min(280, geometry.size.width - 24)).padding(.top, 50)
                            }
                        }
                    } else {
                        HStack(spacing: 0) {
                            SpaceRail(model: model, showLogin: { sheet = .login }, create: { sheet = .createSpace })
                                .frame(width: 60)
                            ChannelSidebar(
                                model: model,
                                sheet: $sheet,
                                narrow: narrow,
                                close: { model.navigationOpen = false }
                            )
                            .frame(width: narrow ? nil : CGFloat(min(sidebarWidth, sidebarMaximum(for: geometry.size.width))))
                            .frame(maxWidth: narrow ? .infinity : CGFloat(min(sidebarWidth, sidebarMaximum(for: geometry.size.width))))
                            .overlay(alignment: .trailing) {
                                if !narrow {
                                    Button { sidebarFocused = true } label: {
                                        Rectangle().fill(Color.clear).frame(width: 8).contentShape(Rectangle())
                                    }
                                        .buttonStyle(.plain)
                                        .highPriorityGesture(DragGesture(minimumDistance: 1, coordinateSpace: .global).onChanged { value in
                                            sidebarFocused = true
                                            if sidebarDragStart == nil { sidebarDragStart = sidebarWidth }
                                            resizeSidebar((sidebarDragStart ?? sidebarWidth) + Double(value.translation.width), viewport: geometry.size.width)
                                        }.onEnded { _ in sidebarDragStart = nil })
                                        .simultaneousGesture(TapGesture(count: 2).onEnded {
                                            resizeSidebar(280, viewport: geometry.size.width)
                                            sidebarFocused = true
                                        })
                                        .focusable()
                                        .focused($sidebarFocused)
                                        .accessibilityLabel("Channel sidebar width")
                                        .accessibilityIdentifier("channel-sidebar-resize")
                                        .accessibilityValue("\(Int(min(sidebarWidth, sidebarMaximum(for: geometry.size.width)))) pixels")
                                        .accessibilityHint("Drag to resize. Arrow keys adjust by 10 pixels; Home and End select the bounds. Double-click resets.")
                                        .accessibilityAdjustableAction { direction in
                                            switch direction {
                                            case .increment: resizeSidebar(sidebarWidth + 10, viewport: geometry.size.width)
                                            case .decrement: resizeSidebar(sidebarWidth - 10, viewport: geometry.size.width)
                                            @unknown default: break
                                            }
                                        }
                                        .onKeyPress { press in
                                            switch press.key {
                                            case .leftArrow: resizeSidebar(sidebarWidth - 10, viewport: geometry.size.width)
                                            case .rightArrow: resizeSidebar(sidebarWidth + 10, viewport: geometry.size.width)
                                            case .home: resizeSidebar(220, viewport: geometry.size.width)
                                            case .end: resizeSidebar(sidebarMaximum(for: geometry.size.width), viewport: geometry.size.width)
                                            default: return .ignored
                                            }
                                            return .handled
                                        }
                                }
                            }
                            if !narrow {
                                Group {
                                    if geometry.size.width >= 1100 {
                                        HStack(spacing: 0) {
                                            ConversationStage(model: model, narrow: false, browse: { model.navigationOpen = true }, membersVisible: membersVisible) {
                                                membersPreference = !membersVisible
                                            }
                                            if membersVisible { MemberPresenceView(model: model).frame(width: 220) }
                                        }
                                    } else {
                                        VStack(spacing: 0) {
                                            ConversationStage(model: model, narrow: false, browse: { model.navigationOpen = true }, membersVisible: membersVisible) {
                                                membersPreference = !membersVisible
                                            }
                                            if membersVisible { MemberPresenceView(model: model).frame(maxHeight: 240) }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(CaperTheme.blackout)
            .overlay(alignment: .top) {
                if model.busy || model.openingSpaceID != nil { ProgressView().progressViewStyle(.linear) }
            }
        }
        .modifier(LoginPresentation(sheet: $sheet, model: model))
        .sheet(item: modalSheet) { item in
            WorkspaceSheetView(item: item, model: model) { sheet = nil }
                .presentationBackground(CaperTheme.surface)
        }
        .task(id: model.detail?.space.id) {
            guard sheet == nil, model.detail != nil else { return }
            if parityFixture == "manage-space" || parityFixture == "modal" { sheet = .manageSpace }
            else if parityFixture == "manage-channel", let channel = model.detail?.channels.first(where: { $0.private }) { sheet = .manageChannel(channel) }
            else if parityFixture == "voice-roster" { CaperRuntime.showVoiceRosterPreview(model) }
        }
    }

    private func sidebarMaximum(for viewport: CGFloat) -> Double {
        max(220, min(440, Double(viewport) - 60 - 320))
    }

    private func resizeSidebar(_ value: Double, viewport: CGFloat) {
        sidebarWidth = min(sidebarMaximum(for: viewport), max(220, value.rounded()))
    }

    private var modalSheet: Binding<WorkspaceSheet?> {
        Binding(get: { if case .login = sheet { return nil }; return sheet }, set: { sheet = $0 })
    }
}

private struct LoginPresentation: ViewModifier {
    @Binding var sheet: WorkspaceSheet?
    @Bindable var model: AppModel
    private var presented: Binding<Bool> {
        Binding(get: { if case .login = sheet { return true }; return false }, set: { if !$0 { sheet = nil } })
    }

    @ViewBuilder func body(content: Content) -> some View {
        #if os(iOS)
        content.fullScreenCover(isPresented: presented) {
            LoginPage(model: model) { sheet = nil }.presentationBackground(CaperTheme.blackout)
        }
        #else
        content.overlay {
            if case .login = sheet { LoginPage(model: model) { sheet = nil } }
        }
        #endif
    }
}

private struct SpaceRail: View {
    @Bindable var model: AppModel
    let showLogin: () -> Void
    let create: () -> Void
    var body: some View {
        ScrollView {
            VStack(spacing: 10) {
                ForEach(model.spaces) { space in
                    Button { Task { await model.select(space: space) } } label: {
                        Text(String(space.name.prefix(1)).uppercased())
                            .font(CaperTheme.font(13, weight: .black))
                            .frame(width: 40, height: 40)
                            .background(model.selectedSpaceID == space.id ? Color(red: 57/255, green: 35/255, blue: 30/255) : CaperTheme.surface)
                            .clipShape(RoundedRectangle(cornerRadius: model.selectedSpaceID == space.id ? 8 : 12))
                            .overlay(RoundedRectangle(cornerRadius: model.selectedSpaceID == space.id ? 8 : 12).stroke(model.selectedSpaceID == space.id ? Color(red: 128/255, green: 81/255, blue: 67/255) : CaperTheme.border))
                    }
                    .buttonStyle(.plain).help(space.name)
                    .modifier(NavigationPrefetchModifier { model.prefetch(space: space) })
                    .accessibilityLabel(space.name)
                    .accessibilityValue(model.openingSpaceID == space.id ? "Opening" : model.selectedSpaceID == space.id ? "Selected" : "")
                    .overlay(alignment: .leading) {
                        if model.selectedSpaceID == space.id {
                            RoundedRectangle(cornerRadius: 2).fill(CaperTheme.terracottaBright).frame(width: 3, height: 24).offset(x: -10)
                        }
                    }
                }
                Button(action: model.account == nil ? showLogin : create) {
                    CaperIcon(name: "plus", size: 20).foregroundStyle(CaperTheme.terracottaBright)
                        .frame(width: 40, height: 40)
                        .background(CaperTheme.surface)
                        .clipShape(RoundedRectangle(cornerRadius: 12))
                        .overlay(RoundedRectangle(cornerRadius: 12).stroke(CaperTheme.border, style: StrokeStyle(lineWidth: 1, dash: [4])))
                }
                .buttonStyle(.plain).disabled(model.account != nil && !model.canCreateSpace)
                .help(model.account == nil ? "Sign in to create a space" : "Create space")
            }.padding(.vertical, 14).frame(maxWidth: .infinity)
        }
        .background(CaperTheme.blackout)
        .overlay(alignment: .trailing) { Rectangle().fill(CaperTheme.border).frame(width: 1) }
    }
}

private struct ChannelSidebar: View {
    @Bindable var model: AppModel
    @Binding var sheet: WorkspaceSheet?
    let narrow: Bool
    let close: () -> Void
    @State private var channelsExpanded = true
    var body: some View {
        VStack(spacing: 0) {
                    HStack(spacing: 6) {
                        Menu {
                            if model.isOwner { Button("Space settings") { sheet = .manageSpace } }
                            else if model.account != nil && model.detail?.space.demo != true { Button("Leave space…", role: .destructive) { sheet = .leaveSpace } }
                        } label: {
                            HStack {
                                Text(model.detail?.space.name ?? "Caper").font(CaperTheme.font(15, weight: .bold)).lineLimit(1)
                                Spacer(); if model.detail?.space.demo != true { CaperIcon(name: "chevron-down") }
                            }.contentShape(Rectangle())
                        }.menuStyle(.borderlessButton).disabled(model.detail?.space.demo == true)
                            .accessibilityLabel(model.detail?.space.name ?? "Caper")
                            .accessibilityIdentifier("selected-space-name")
                        if narrow { Button(action: close) { CaperIcon(name: "x") }.buttonStyle(SidebarIconButton()).accessibilityLabel("Close navigation") }
                    }
                    .padding(.horizontal, 16).frame(height: 50)
                    .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    HStack {
                        Button { channelsExpanded.toggle(); CaperEffects.shared.toggle(channelsExpanded) } label: {
                            HStack(spacing: 6) {
                                CaperIcon(name: channelsExpanded ? "chevron-down" : "chevron-right")
                                Text("Channels")
                            }.font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                        }.buttonStyle(.plain)
                        Spacer()
                        if model.isOwner {
                            Button { sheet = .createChannel } label: { CaperIcon(name: "plus") }
                                .buttonStyle(SidebarIconButton()).disabled(!model.canCreateChannel).accessibilityLabel("Create channel")
                        }
                    }.frame(height: 44)

                    if channelsExpanded {
                        VStack(spacing: 3) {
                            ForEach(model.detail?.channels ?? []) { channel in
                                HStack(spacing: 2) {
                                    Button { Task { await model.select(channel: channel) } } label: {
                                        HStack(spacing: 9) {
                                            CaperIcon(name: channel.private ? "lock" : "hash", size: 18)
                                                .foregroundStyle(model.selectedChannelID == channel.id ? CaperTheme.terracottaBright : CaperTheme.muted)
                                            Text(channel.name).lineLimit(1)
                                            Spacer()
                                        }
                                        .font(CaperTheme.font(13, weight: .medium))
                                        .foregroundStyle(model.selectedChannelID == channel.id ? CaperTheme.text : CaperTheme.muted)
                                        .padding(.horizontal, 9).frame(height: 38)
                                        .background(model.selectedChannelID == channel.id ? CaperTheme.terracotta.opacity(0.16) : Color.clear)
                                        .clipShape(RoundedRectangle(cornerRadius: 6))
                                    }.buttonStyle(.plain)
                                        .modifier(NavigationPrefetchModifier {
                                            if let space = model.detail?.space { model.prefetch(space: space, channelID: channel.id) }
                                        })
                                        .accessibilityIdentifier("channel-\(channel.id)")
                                        .accessibilityValue(model.openingChannelID == channel.id ? "Opening" : model.selectedChannelID == channel.id ? "Selected" : "")
                                    if model.isOwner {
                                        Button { sheet = .manageChannel(channel) } label: { CaperIcon(name: "settings") }
                                            .buttonStyle(SidebarIconButton()).help("Manage \(channel.name)").accessibilityLabel("Manage \(channel.name)")
                                    }
                                }
                                ChannelVoiceSlot(model: model, channel: channel)
                            }
                        }
                    }

                    if let error = model.error {
                        Text(error).font(CaperTheme.font(11)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)).padding(8)
                    }
                    if (model.voice.phase == .connected || model.voice.phase == .reconnecting || !model.voice.participants.isEmpty),
                       (!channelsExpanded || !((model.detail?.channels ?? []).contains { $0.id == model.voice.context?.channelID })) {
                        VoiceRoster(model: model)
                    }
                }.padding(.horizontal, 16)
            }
            AccountBar(model: model, sheet: $sheet)
        }
        .background(CaperTheme.sidebar)
        .overlay(alignment: .trailing) { Rectangle().fill(CaperTheme.border).frame(width: 1) }
    }
}

private struct NavigationPrefetchModifier: ViewModifier {
    let action: () -> Void
    #if os(macOS)
    @FocusState private var focused: Bool
    #endif

    func body(content: Content) -> some View {
        #if os(macOS)
        content
            .onHover { if $0 { action() } }
            .focused($focused)
            .onChange(of: focused) { _, value in if value { action() } }
        #else
        content
        #endif
    }
}

private struct SidebarIconButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.system(size: 14, weight: .semibold)).foregroundStyle(CaperTheme.muted)
            .frame(width: 28, height: 28).background(configuration.isPressed ? CaperTheme.border : .clear)
            .clipShape(RoundedRectangle(cornerRadius: 5))
    }
}

private struct ChannelVoiceSlot: View {
    @Bindable var model: AppModel
    let channel: Channel
    @State private var collapsed = false

    private var active: Bool {
        model.voice.context?.channelID == channel.id && model.voice.phase != .idle && model.voice.phase != .failed
    }
    private var people: [VoiceSpectator] {
        if active {
            return model.voice.participants.map {
                VoiceSpectator(id: $0.id, name: $0.name, muted: $0.muted,
                               deafened: $0.deafened, countryCode: $0.countryCode)
            }
        }
        return model.voicePresence.roster(for: channel.id)
    }

    var body: some View {
        let occupants = people
        if !occupants.isEmpty || model.selectedChannelID == channel.id || active {
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 8) {
                    if !occupants.isEmpty {
                        Button { collapsed.toggle() } label: {
                            HStack(spacing: 5) {
                                ForEach(occupants.prefix(3)) { person in
                                    Avatar(name: person.name, size: 20)
                                }
                                if occupants.count > 3 { Text("+\(occupants.count - 3)") }
                                CaperIcon(name: collapsed ? "chevron-right" : "chevron-down", size: 12)
                            }
                        }.buttonStyle(.plain)
                            .accessibilityLabel("\(occupants.count) in voice in \(channel.name). \(collapsed ? "Show" : "Hide") who is in voice")
                            .accessibilityIdentifier("voice-stack-\(channel.id)")
                            .accessibilityValue(collapsed ? "Collapsed" : "Expanded")
                    }
                    Spacer(minLength: 0)
                    if !active {
                        Button("Join") { Task { await model.joinVoice(channel: channel) } }
                            .buttonStyle(VoiceJoinButton())
                            .accessibilityLabel(model.voice.phase == .idle || model.voice.phase == .failed
                                ? "Join voice in #\(channel.name)" : "Switch voice to #\(channel.name)")
                            .accessibilityIdentifier("join-voice-\(channel.id)")
                    }
                }
                if active {
                    if !collapsed { VoiceRoster(model: model) }
                } else if !collapsed {
                    ForEach(occupants) { person in
                        HStack(spacing: 7) {
                            Avatar(name: person.name, size: 23)
                            Text(person.name).lineLimit(1)
                            if person.muted { CaperIcon(name: "mic-off", size: 13) }
                            if person.deafened { CaperIcon(name: "volume-x", size: 13) }
                        }.font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                            .accessibilityElement(children: .combine)
                    }
                }
            }.padding(.leading, 34).padding(.trailing, 8).padding(.bottom, 5)
        }
    }
}

private struct MemberPresenceView: View {
    @Bindable var model: AppModel
    @Bindable var presence: PresenceModel
    init(model: AppModel) { self.model = model; presence = model.presence }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Members").font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                Spacer()
                if model.detail?.space.demo != true {
                    Text("\(model.detail?.members.count ?? 0)").font(CaperTheme.font(10, weight: .bold)).foregroundStyle(CaperTheme.muted)
                }
            }.padding(.horizontal, 12).frame(height: 50)
                .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
            VStack(alignment: .leading, spacing: 8) {
            if model.detail?.space.demo == true {
                Text("General is open to everyone. People in voice appear in the channel sidebar.")
                    .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
            } else if model.detail?.members.isEmpty != false {
                Text("No members to show.").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
            } else {
                ForEach(presence.visibleMembers) { member in
                    HStack(spacing: 10) {
                        ZStack(alignment: .bottomTrailing) {
                            Avatar(name: member.displayName, size: 30)
                            Circle().fill(statusColor(presence.status(for: member))).frame(width: 10, height: 10)
                                .overlay(Circle().stroke(CaperTheme.sidebar, lineWidth: 2))
                        }
                        Text(member.displayName).font(CaperTheme.font(12, weight: .medium)).lineLimit(1)
                    }.frame(height: 38)
                }
                if presence.pageCount > 1 {
                    HStack {
                        Button("Previous") { Task { await presence.showPage(presence.page - 1) } }.disabled(presence.page == 0)
                        Spacer(); Text("\(presence.page + 1) / \(presence.pageCount)")
                        Spacer(); Button("Next") { Task { await presence.showPage(presence.page + 1) } }.disabled(presence.page + 1 >= presence.pageCount)
                    }.font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
                }
            }
            }.padding(.horizontal, 12)
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(CaperTheme.sidebar)
            .overlay(alignment: .leading) { Rectangle().fill(CaperTheme.border).frame(width: 1) }
    }
    private func statusColor(_ status: PresenceStatus) -> Color {
        switch status { case .online: CaperTheme.green; case .idle: Color(red: 0.72, green: 0.60, blue: 0.35); case .offline, .unknown: CaperTheme.border }
    }
}

private struct VoiceRoster: View {
    @Bindable var voice: VoiceClient
    @State private var audioParticipantID: String? = nil
    init(model: AppModel) { voice = model.voice }
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(voice.participants) { participant in
                HStack(spacing: 8) {
                    Avatar(name: participant.name, size: 20)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(participant.name + (voice.isSelf(participantID: participant.id) ? " (you)" : ""))
                            .font(CaperTheme.font(12, weight: .medium)).lineLimit(1)
                        if !voice.isSelf(participantID: participant.id), voice.locallyMutedParticipants.contains(participant.id) {
                            HStack(spacing: 4) {
                                CaperIcon(name: "volume-x", size: 10)
                                Text("You muted \(participant.name)")
                                    .accessibilityIdentifier("participant-local-muted-\(participant.id)")
                            }.font(CaperTheme.font(10)).foregroundStyle(CaperTheme.terracottaBright)
                        }
                    }
                    Spacer(minLength: 0)
                    if (voice.isSelf(participantID: participant.id) ? voice.muted : participant.muted) {
                        CaperIcon(name: "mic-off", size: 14).accessibilityHidden(false).accessibilityLabel("Muted")
                    }
                    if (voice.isSelf(participantID: participant.id) ? voice.deafened : participant.deafened) {
                        CaperIcon(name: "volume-x", size: 14).accessibilityHidden(false).accessibilityLabel("Deafened")
                    }
                    if !voice.isSelf(participantID: participant.id) {
                        Button { audioParticipantID = audioParticipantID == participant.id ? nil : participant.id } label: {
                            Text("Audio").font(CaperTheme.font(10, weight: .bold))
                        }.buttonStyle(.plain)
                            .accessibilityLabel("Audio controls for \(participant.name)")
                            .accessibilityIdentifier("participant-audio-\(participant.id)")
                            .popover(isPresented: Binding(
                                get: { audioParticipantID == participant.id },
                                set: { if !$0, audioParticipantID == participant.id { audioParticipantID = nil } }
                            ), arrowEdge: .trailing) {
                                VStack(alignment: .leading, spacing: 12) {
                                    Text("User volume · \(voice.participantGains[participant.id] ?? 100)%")
                                        .font(CaperTheme.font(12, weight: .bold))
                                    Slider(value: participantGain(participant.id), in: 0...200, step: 1)
                                        .accessibilityLabel("\(participant.name) volume")
                                    Toggle("Mute", isOn: participantMute(participant.id))
                                        .font(CaperTheme.font(12, weight: .medium))
                                }.padding(16).frame(width: 220).background(CaperTheme.surface)
                                #if os(iOS)
                                .presentationCompactAdaptation(.popover)
                                #endif
                            }
                    }
                }.foregroundStyle(CaperTheme.muted).padding(.vertical, 4)
            }
        }
    }

    private func participantGain(_ id: String) -> Binding<Double> {
        Binding(
            get: { Double(voice.participantGains[id] ?? 100) },
            set: { voice.setParticipantGain(Int($0), participantID: id); CaperEffects.shared.slider($0 / 200) }
        )
    }

    private func participantMute(_ id: String) -> Binding<Bool> {
        Binding(
            get: { voice.locallyMutedParticipants.contains(id) },
            set: { voice.setParticipantMuted($0, participantID: id); CaperEffects.shared.toggle(!$0) }
        )
    }
}

private struct VoiceJoinButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.terracottaBright)
            .padding(.horizontal, 10).frame(height: 32).background(CaperTheme.terracotta.opacity(configuration.isPressed ? 0.25 : 0.14))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(CaperTheme.terracotta.opacity(0.7))).clipShape(RoundedRectangle(cornerRadius: 6))
    }
}

private struct AccountBar: View {
    @Bindable var model: AppModel
    @Bindable var voice: VoiceClient
    @Binding var sheet: WorkspaceSheet?
    #if os(macOS)
    @State private var inputOptions = false
    @State private var outputOptions = false
    @State private var audioDiagnostics = false
    #endif
    init(model: AppModel, sheet: Binding<WorkspaceSheet?>) { self.model = model; voice = model.voice; _sheet = sheet }
    var body: some View {
        VStack(spacing: 0) {
            if let context = voice.context, voice.phase != .idle && voice.phase != .failed {
                HStack(spacing: 8) {
                    Button { Task { await model.openVoiceContext() } } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(voice.phase == .connected ? "Voice connected" : "Connecting voice…")
                                .font(CaperTheme.font(11, weight: .bold))
                            Text("\(context.channelName) / \(context.spaceName)")
                                .font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }.buttonStyle(.plain)
                    Button { if voice.phase == .connected { CaperEffects.shared.play(.leave) }; model.leaveVoice() } label: { CaperIcon(name: "x") }
                        .buttonStyle(SidebarIconButton()).accessibilityLabel("Disconnect voice")
                }.padding(9).background(CaperTheme.raised).clipShape(RoundedRectangle(cornerRadius: 8))
                    .accessibilityIdentifier("active-voice-context")
            }
            HStack(spacing: 5) {
            Button { sheet = model.account == nil ? .login : .profile } label: {
                HStack(spacing: 7) {
                    Avatar(name: model.account?.displayName ?? "Guest", size: 30)
                    Text(model.account?.displayName ?? "Sign in").font(CaperTheme.font(13, weight: .medium)).lineLimit(1)
                    Spacer()
                }.contentShape(Rectangle())
            }.buttonStyle(.plain)
                .accessibilityIdentifier("account-profile")
            Button { CaperEffects.shared.toggle(voice.muted); Task { await voice.setMuted(!voice.muted) } } label: {
                CaperIcon(name: voice.muted ? "mic-off" : "mic", size: 20)
                    .foregroundStyle(voice.muted ? CaperTheme.terracottaBright : CaperTheme.muted)
            }.buttonStyle(SidebarIconButton()).accessibilityLabel("Microphone")
                .accessibilityValue(voice.muted ? "Muted" : "On").accessibilityIdentifier("microphone-toggle")
            #if os(macOS)
            Button { inputOptions.toggle(); CaperEffects.shared.toggle(inputOptions) } label: {
                CaperIcon(name: "chevron-down", size: 12).frame(width: 14, height: 28)
            }.buttonStyle(.plain).accessibilityLabel("Input Options")
                .popover(isPresented: $inputOptions, arrowEdge: .top) { AccountAudioMenu(voice: voice, input: true) }
            #endif
            Button { CaperEffects.shared.toggle(voice.deafened); Task { await voice.setDeafened(!voice.deafened) } } label: {
                CaperIcon(name: voice.deafened ? "volume-x" : "headphones", size: 20)
                    .foregroundStyle(voice.deafened ? CaperTheme.terracottaBright : CaperTheme.muted)
            }.buttonStyle(SidebarIconButton()).accessibilityLabel("Headphones")
                .accessibilityValue(voice.deafened ? "Deafened" : "On").accessibilityIdentifier("deafen-toggle")
            #if os(macOS)
            Button { outputOptions.toggle(); CaperEffects.shared.toggle(outputOptions) } label: {
                CaperIcon(name: "chevron-down", size: 12).frame(width: 14, height: 28)
            }.buttonStyle(.plain).accessibilityLabel("Output Options")
                .popover(isPresented: $outputOptions, arrowEdge: .top) { AccountAudioMenu(voice: voice, input: false) }
            #endif
            Menu {
                Button("Audio preferences") { voice.showAudioPreferences = true }
                #if os(macOS)
                if model.account?.debugEnabled == true {
                    Button("Audio diagnostics") { audioDiagnostics = true }
                }
                #endif
                if model.account != nil { Button("Log out", role: .destructive) { Task { await model.logout() } } }
            } label: { CaperIcon(name: "settings", size: 20) }.menuStyle(.borderlessButton).frame(width: 28)
                .accessibilityLabel("Account settings").accessibilityIdentifier("account-settings-menu")
            }.padding(4).frame(height: 42).background(CaperTheme.raised)
        }
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(CaperTheme.border)).clipShape(RoundedRectangle(cornerRadius: 6))
        .padding(12)
        .sheet(isPresented: $voice.showAudioPreferences) { AudioPreferencesView(voice: voice, debugEnabled: model.account?.debugEnabled == true).presentationBackground(CaperTheme.surface) }
        #if os(macOS)
        .sheet(isPresented: $audioDiagnostics) {
            VStack(alignment: .leading, spacing: 18) {
                SheetHeader(title: "Audio diagnostics", detail: "Local processing counters", close: { audioDiagnostics = false })
                if model.account?.debugEnabled == true { AudioDiagnosticsView(voice: voice).padding(22) }
            }.frame(minWidth: 420).background(CaperTheme.surface)
        }
        #endif
    }
}

#if os(macOS)
private struct AccountAudioMenu: View {
    @Bindable var voice: VoiceClient
    let input: Bool
    @Environment(\.dismiss) private var dismiss
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(input ? "Microphone" : "Audio output").font(CaperTheme.font(13, weight: .bold))
            Picker("Device", selection: Binding(get: { (input ? voice.selectedInputID : voice.selectedOutputID) ?? "" }, set: { uid in
                let changed = input ? voice.selectInput(uid) : voice.selectOutput(uid)
                error = changed ? nil : "Could not switch devices. Check system audio settings."
            })) {
                Text("System default").tag("")
                ForEach(input ? voice.availableInputs : voice.availableOutputs) { device in
                    Text(device.name).tag(device.id)
                }
            }.labelsHidden()
            if let error { Text(error).font(CaperTheme.font(11)).foregroundStyle(.red) }
            if input {
                Text("Input volume · \(voice.inputGain)%").font(CaperTheme.font(12))
                Slider(value: Binding(get: { Double(voice.inputGain) }, set: { voice.setInputGain(Int($0)); CaperEffects.shared.slider($0 / 200) }), in: 0...200, step: 1)
                    .accessibilityLabel("Input volume")
                Text("Voice processing · \(voice.voiceProcessingStrength)%").font(CaperTheme.font(12))
                Slider(value: Binding(get: { Double(voice.voiceProcessingStrength) }, set: { voice.setVoiceProcessingStrength(Int($0)); CaperEffects.shared.slider($0 / 100) }), in: 0...100, step: 1)
                    .accessibilityLabel("Voice processing")
            } else {
                Text("Output volume · \(voice.outputGain)%").font(CaperTheme.font(12))
                Slider(value: Binding(get: { Double(voice.outputGain) }, set: { voice.setOutputGain(Int($0)); CaperEffects.shared.slider($0 / 200) }), in: 0...200, step: 1)
                    .accessibilityLabel("Output volume")
            }
            Button("Audio preferences") { dismiss(); voice.showAudioPreferences = true }
        }.padding(16).frame(width: 260).background(CaperTheme.surface)
            .task { await voice.refreshAudioDevices() }
    }
}
#endif

private struct Avatar: View {
    let name: String; let size: CGFloat
    var body: some View {
        Text(String(name.prefix(1)).uppercased()).font(CaperTheme.font(size * 0.36, weight: .black))
            .frame(width: size, height: size).background(CaperTheme.raised).clipShape(Circle())
    }
}

private struct ConversationStage: View {
    @Bindable var model: AppModel
    let narrow: Bool
    let browse: () -> Void
    let membersVisible: Bool
    let toggleMembers: () -> Void
    var body: some View {
        if model.selectedChannelID == nil {
            VStack(spacing: 8) {
                Button(action: browse) { Label("Browse spaces", systemImage: "number") }.buttonStyle(.bordered)
                CaperIcon(name: "hash", size: 30).foregroundStyle(CaperTheme.terracottaBright)
                Text("No accessible channels").font(CaperTheme.font(20, weight: .bold))
                Text(model.isOwner ? "Create a channel to start a conversation." : "The owner has not shared a channel with you yet.")
                    .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
            }.frame(maxWidth: .infinity, maxHeight: .infinity).background(CaperTheme.conversation)
        } else { ChatView(model: model, narrow: narrow, browse: browse, membersVisible: membersVisible, toggleMembers: toggleMembers) }
    }
}

private struct ChatView: View {
    @Bindable var model: AppModel
    @Bindable var chat: ChatModel
    @Bindable var voice: VoiceClient
    let narrow: Bool
    let browse: () -> Void
    let membersVisible: Bool
    let toggleMembers: () -> Void
    init(model: AppModel, narrow: Bool, browse: @escaping () -> Void, membersVisible: Bool, toggleMembers: @escaping () -> Void) {
        self.model = model; chat = model.chat; voice = model.voice; self.narrow = narrow; self.browse = browse
        self.membersVisible = membersVisible; self.toggleMembers = toggleMembers
    }
    var body: some View {
        VStack(spacing: 0) {
            // Match the 34pt message-avatar column without shrinking the 44pt menu target.
            HStack(spacing: narrow ? 5 : 10) {
                if narrow {
                    Button(action: browse) {
                        CaperIcon(name: "menu", size: 18)
                            .frame(width: 44, height: 44).contentShape(Rectangle())
                    }.buttonStyle(.plain).accessibilityLabel("Open navigation")
                }
                Text("# \(chat.channelName.lowercased())").font(CaperTheme.font(14, weight: .medium)).lineLimit(1)
                    .accessibilityLabel("# \(chat.channelName.lowercased())")
                    .accessibilityIdentifier("selected-channel-name")
                Spacer()
                VoiceHeaderButton(model: model, voice: voice)
                if chat.liveState != .connected { Text(chat.liveState == .reconnecting ? "Reconnecting…" : "Connecting…").font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.muted) }
                Button { CaperEffects.shared.toggle(!membersVisible); toggleMembers() } label: { CaperIcon(name: "users", size: 20) }
                    .buttonStyle(SidebarIconButton()).accessibilityLabel(membersVisible ? "Hide members" : "Show members")
            }.padding(.leading, narrow ? 13 : 18).padding(.trailing, 18).frame(height: 50)
                .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }

            if let error = voice.error {
                Text(error).font(CaperTheme.font(12)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51))
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18).padding(.vertical, 8)
            }
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 0) {
                        HStack {
                            if chat.hasMore {
                                Button(chat.loadingOlder ? "Loading…" : chat.olderError == nil ? "Load older messages" : "Retry older messages") {
                                    Task { await chat.loadOlder() }
                                }.disabled(chat.loadingOlder)
                                    .accessibilityIdentifier("load-older-messages")
                            }
                            else { Text("Beginning of conversation") }
                        }.font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.muted).frame(height: 44)
                        if let error = chat.olderError {
                            Text(error).font(CaperTheme.font(11)).foregroundStyle(.red).padding(.horizontal, 18).padding(.bottom, 8)
                        }
                        ForEach(chat.messages) { message in MessageRow(message: message).id(message.id) }
                        if let pending = chat.pendingMessage {
                            PendingMessageRow(pending: pending, author: chat.currentAuthor, error: chat.error,
                                              rejected: chat.sendRejected, canEdit: chat.draft.isEmpty,
                                              retry: { Task { await chat.send() } },
                                              edit: { _ = chat.discardRejected(edit: true) },
                                              dismiss: { _ = chat.discardRejected() })
                                .id("pending-\(pending.id)")
                        }
                        if chat.messages.isEmpty && !chat.loading && chat.pendingMessage == nil {
                            VStack(spacing: 7) {
                                Text("No messages yet.").font(CaperTheme.font(14, weight: .medium))
                                Text("Start the conversation in #\(chat.channelName.lowercased()).").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                            }.padding(.top, 80)
                        }
                    }
                }
                .accessibilityIdentifier("chat-timeline")
                .onChange(of: chat.messages.last?.id) { _, id in if let id { proxy.scrollTo(id, anchor: .bottom) } }
                .onChange(of: chat.pendingMessage?.id, initial: true) { _, id in
                    if let id { proxy.scrollTo("pending-\(id)", anchor: .bottom) }
                }
            }

            HStack(spacing: 7) {
                if !chat.typingNames.isEmpty {
                    ProgressView().controlSize(.mini)
                    Text(typingLabel).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                }
                Spacer()
            }.padding(.horizontal, 18).frame(height: 20)

            if let error = chat.error, chat.pendingMessage == nil {
                Text(error).font(CaperTheme.font(11)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)).frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18)
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField("Message #\(chat.channelName.lowercased())", text: $chat.draft, axis: .vertical)
                    .font(CaperTheme.font(14)).lineLimit(1...8).textFieldStyle(.plain).padding(11)
                    .background(CaperTheme.composer).clipShape(RoundedRectangle(cornerRadius: 6))
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(CaperTheme.border))
                    .accessibilityIdentifier("message-composer")
                    .accessibilityValue(chat.draft)
                    .accessibilityHint("Return sends on macOS. Shift-Return adds a new line. Use the Send button when composing with the iPhone keyboard.")
                    .onChange(of: chat.draft) { _, value in chat.setTyping(!value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) }
                    .onSubmit { Task { await chat.send() } }
                Button { Task { await chat.send() } } label: {
                    Image(systemName: "arrow.up").font(.system(size: 15, weight: .bold))
                }
                .buttonStyle(PrimaryIconButton())
                .disabled(chat.sending || chat.sendRejected || (chat.pendingMessage == nil && MessageValidation.error(for: chat.draft) != nil))
                .accessibilityLabel("Send message")
                .accessibilityIdentifier("send-message-button")
            }.padding(.horizontal, 18).padding(.vertical, 12)
            if chat.draft.unicodeScalars.count >= 3000 {
                Text("\(chat.draft.unicodeScalars.count.formatted()) / 4,000").font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted).padding(.bottom, 6)
            }
        }.background(CaperTheme.conversation)
    }
    private var typingLabel: String {
        if chat.typingNames.count > 2 { return "Several people are typing…" }
        let names = chat.typingNames.joined(separator: " and ")
        return "\(names) \(chat.typingNames.count == 1 ? "is" : "are") typing…"
    }
}

private struct VoiceHeaderButton: View {
    let model: AppModel
    @Bindable var voice: VoiceClient
    var body: some View {
        if sameChannel, voice.phase == .joining || voice.phase == .reconnecting {
            Button { model.leaveVoice() } label: { HStack(spacing: 7) { CaperIcon(name: "speech"); Text("Cancel") } }.buttonStyle(VoiceJoinButton())
        } else if sameChannel, voice.phase == .connected {
            Button { CaperEffects.shared.play(.leave); model.leaveVoice() } label: { HStack(spacing: 7) { CaperIcon(name: "speech"); Text("Leave") } }.buttonStyle(VoiceJoinButton())
        } else if voice.phase == .leaving {
            ProgressView().controlSize(.small)
        } else {
            Button(action: joinSelectedChannel) { HStack(spacing: 7) { CaperIcon(name: "speech"); Text("Join") } }
                .buttonStyle(VoiceJoinButton()).disabled(selectedContext == nil)
                .accessibilityIdentifier("join-voice-button")
        }
    }

    private var selectedContext: VoiceContext? {
        guard let detail = model.detail,
              let channelID = model.selectedChannelID,
              let channel = detail.channels.first(where: { $0.id == channelID }) else { return nil }
        return VoiceContext(channelID: channel.id, channelName: channel.name, spaceID: detail.space.id, spaceName: detail.space.name)
    }

    private var sameChannel: Bool {
        guard let selectedContext else { return false }
        return voice.context?.channelID == selectedContext.channelID
    }

    private func joinSelectedChannel() {
        guard let channel = model.detail?.channels.first(where: { $0.id == model.selectedChannelID }) else { return }
        Task { await model.joinVoice(channel: channel) }
    }
}

private struct PrimaryIconButton: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.foregroundStyle(.white).frame(width: 42, height: 42)
            .background(configuration.isPressed ? CaperTheme.terracottaBright : CaperTheme.terracotta)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .opacity(isEnabled ? 1 : 0.45)
    }
}

private struct MessageRow: View {
    let message: ChatMessage
    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Avatar(name: message.author.name, size: 34)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 7) {
                    Text(message.author.name).font(CaperTheme.font(13, weight: .bold))
                    if message.author.isGuest { Text("GUEST").font(CaperTheme.font(9, weight: .bold)).foregroundStyle(CaperTheme.muted).padding(.horizontal, 5).overlay(RoundedRectangle(cornerRadius: 4).stroke(CaperTheme.border)) }
                    Text(timeLabel(message.createdAt)).font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
                }
                Text(message.content.text).font(CaperTheme.font(14)).foregroundStyle(Color(red: 222/255, green: 223/255, blue: 224/255)).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }.padding(.horizontal, 18).padding(.vertical, 10)
    }
    private func timeLabel(_ value: String) -> String {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let date = fractional.date(from: value) ?? ISO8601DateFormatter().date(from: value) else { return "" }
        return date.formatted(date: .omitted, time: .shortened)
    }
}

private struct PendingMessageRow: View {
    let pending: PendingMessage; let author: ChatAuthor?; let error: String?
    let rejected: Bool; let canEdit: Bool; let retry: () -> Void; let edit: () -> Void; let dismiss: () -> Void
    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Avatar(name: author?.name ?? "Guest", size: 34)
            VStack(alignment: .leading, spacing: 4) {
                Text(author?.name ?? "Guest").font(CaperTheme.font(13, weight: .bold))
                Text(pending.text).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted)
                if let error {
                    VStack(alignment: .leading, spacing: 5) {
                        Text("\(rejected ? "Not sent." : "Not confirmed yet.") \(error)")
                        HStack(spacing: 12) {
                            if rejected {
                                Button("Edit", action: edit).disabled(!canEdit)
                                Button("Dismiss", action: dismiss)
                            } else { Button("Retry send", action: retry) }
                        }
                    }
                    .font(CaperTheme.font(11, weight: .medium)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51))
                }
            }
        }.padding(.horizontal, 18).padding(.vertical, 10)
    }
}

private struct ProfileView: View {
    @Bindable var model: AppModel
    @State private var username = ""
    @State private var displayName = ""
    var body: some View {
        VStack(spacing: 22) {
            Wordmark(); Text("Finish your profile").font(CaperTheme.font(28, weight: .bold))
            CaperField(title: "Username", text: $username)
            Text("3-32 lowercase letters, numbers, or underscores.")
                .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
            CaperField(title: "Display name", text: $displayName)
            Text("Shown to other people. It does not need to be unique.")
                .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
            if let error = model.error { Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.terracottaBright) }
            Button(model.busy ? "Saving…" : "Continue") { Task { await model.saveProfile(username: username, displayName: displayName) } }
                .buttonStyle(CaperPrimaryButton())
                .disabled(model.busy || ProfileValidation.error(username: username, displayName: displayName) != nil)
                .accessibilityIdentifier("profile-continue")
        }.padding(28).frame(maxWidth: 440).frame(maxWidth: .infinity, maxHeight: .infinity).background(CaperTheme.blackout)
    }
}

private struct WorkspaceSheetView: View {
    let item: WorkspaceSheet
    @Bindable var model: AppModel
    let close: () -> Void
    var body: some View {
        Group {
            switch item {
            case .login: LoginSheet(model: model, close: close)
            case .profile: ProfileSheet(model: model, close: close)
            case .createSpace: SpaceEditor(model: model, close: close, managing: false)
            case .createChannel: ChannelEditor(model: model, channel: nil, close: close)
            case .manageSpace: SpaceEditor(model: model, close: close, managing: true)
            case .manageChannel(let channel): ChannelEditor(model: model, channel: channel, close: close)
            case .leaveSpace: ConfirmationSheet(title: "Leave \(model.detail?.space.name ?? "space")?", detail: "You will lose access to its channels and conversations.", action: "Leave space", close: close) { try await model.leaveCurrentSpace() }
            }
        }.frame(minWidth: 320, idealWidth: item.id.contains("manage") ? 600 : 460)
    }
}

private struct SheetHeader: View {
    let title: String; var detail: String?; let close: () -> Void
    var body: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 6) {
                Text(title).font(CaperTheme.font(20, weight: .bold))
                if let detail { Text(detail).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted) }
            }
            Spacer(); Button(action: close) { CaperIcon(name: "x") }.buttonStyle(SidebarIconButton()).accessibilityLabel("Close")
        }.padding(22).overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
    }
}

private struct LoginSheet: View {
    @Bindable var model: AppModel; let close: () -> Void
    @State private var email = ""; @State private var code = ""
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: "Sign in to Caper", detail: model.challengeID == nil ? "We’ll email you a short verification code." : "Enter the code from your email.", close: close)
            VStack(spacing: 16) {
                if model.challengeID == nil {
                    CaperField(title: "Email", text: $email)
                    Button("Email me a code") { Task { await model.requestCode(email: email) } }.buttonStyle(CaperPrimaryButton()).disabled(model.busy || email.isEmpty)
                } else {
                    CaperField(title: "Verification code", text: $code)
                    Button("Verify") { Task { await model.verify(code: code); if model.phase != .onboarding { close() } } }.buttonStyle(CaperPrimaryButton()).disabled(model.busy || code.isEmpty)
                    Button("Use another email") { model.challengeID = nil }.buttonStyle(.plain).foregroundStyle(CaperTheme.muted)
                }
                if let error = model.error { Text(error).font(CaperTheme.font(12)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)) }
            }.padding(22)
        }.background(CaperTheme.surface)
    }
}

private struct LoginPage: View {
    @Bindable var model: AppModel
    let close: () -> Void
    @State private var email = ""
    @State private var code = ""
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Wordmark().padding(.bottom, 58)
                Text("WELCOME TO CAPER").font(CaperTheme.font(12, weight: .bold)).tracking(2).foregroundStyle(CaperTheme.muted).padding(.bottom, 24)
                Text(model.challengeID == nil ? "Come on in." : "Check your email.")
                    .font(CaperTheme.font(52, weight: .black)).tracking(-2.5).padding(.bottom, 18)
                Text(model.challengeID == nil ? "Use your email to create an account or return to one. No password needed." : "Enter the six-character code we sent you.")
                    .font(CaperTheme.font(16)).foregroundStyle(CaperTheme.muted).lineSpacing(7).padding(.bottom, 30)
                if model.challengeID == nil {
                    CaperField(title: "Email address", text: $email)
                    if let error = model.error { LoginError(message: error).padding(.top, 18) }
                    Button { Task { await model.requestCode(email: email) } } label: {
                        HStack { Text(model.busy ? "Sending…" : "Email me a code"); Spacer(); Image(systemName: "arrow.right") }
                    }.buttonStyle(LoginActionButton()).disabled(model.busy || email.isEmpty).padding(.top, model.error == nil ? 28 : 28)
                    Button(action: close) {
                        (Text("We only send a code when you ask. Prefer to look around first? ")
                            .foregroundStyle(CaperTheme.muted)
                         + Text("Join general as a guest.").fontWeight(.bold).foregroundStyle(CaperTheme.text))
                            .font(CaperTheme.font(14)).multilineTextAlignment(.leading)
                    }.buttonStyle(.plain).padding(.top, 22)
                        .accessibilityIdentifier("guest-general-button")
                } else {
                    CaperField(title: "Verification code", text: $code)
                    if let error = model.error { LoginError(message: error).padding(.top, 18) }
                    Button { Task { await model.verify(code: code); if model.phase != .onboarding { close() } } } label: {
                        HStack { Text(model.busy ? "Verifying…" : "Continue"); Spacer(); Image(systemName: "arrow.right") }
                    }.buttonStyle(LoginActionButton()).disabled(model.busy || code.isEmpty).padding(.top, 28)
                    Button("Use another email") { model.challengeID = nil; model.error = nil }.buttonStyle(.plain).foregroundStyle(CaperTheme.muted).padding(.top, 18)
                }
            }
            .frame(width: 440)
            .padding(.top, 108)
            .frame(maxWidth: .infinity, alignment: .center)
        }.background(CaperTheme.blackout.ignoresSafeArea())
    }
}

private struct LoginError: View {
    let message: String
    var body: some View {
        Text(message).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.text).padding(.horizontal, 14)
            .frame(maxWidth: .infinity, minHeight: 50, alignment: .leading)
            .background(CaperTheme.surface)
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(CaperTheme.terracottaBright))
    }
}

private struct LoginActionButton: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(CaperTheme.font(16, weight: .medium)).foregroundStyle(.white)
            .padding(.horizontal, 20).frame(maxWidth: .infinity).frame(height: 58)
            .background(configuration.isPressed ? CaperTheme.terracottaBright : CaperTheme.terracotta)
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .opacity(isEnabled ? 1 : 0.45)
    }
}

private struct ProfileSheet: View {
    @Bindable var model: AppModel; let close: () -> Void
    @State private var username = ""
    @State private var displayName = ""
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: "Account", detail: model.account?.username.map { "@\($0)" }, close: close)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack { Avatar(name: model.account?.displayName ?? "Caper", size: 42); Text(model.account?.displayName ?? "Caper").font(CaperTheme.font(16, weight: .bold)) }
                    CaperField(title: "Username", text: $username)
                    CaperField(title: "Display name", text: $displayName)
                    if let error = model.error { Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.terracottaBright) }
                    Button(model.busy ? "Saving…" : "Save profile") {
                        Task {
                            await model.saveProfile(username: username, displayName: displayName)
                            if model.error == nil { close() }
                        }
                    }.buttonStyle(CaperPrimaryButton())
                        .disabled(model.busy || ProfileValidation.error(username: username, displayName: displayName) != nil)
                        .accessibilityIdentifier("profile-save")
                    Button("Log out", role: .destructive) { Task { await model.logout(); close() } }.buttonStyle(.bordered)
                }.padding(22).frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxHeight: 500).scrollDismissesKeyboard(.interactively)
        }.background(CaperTheme.surface)
            .onAppear {
                username = model.account?.username ?? ""
                displayName = model.account?.displayName ?? ""
            }
    }
}

private struct SpaceEditor: View {
    @Bindable var model: AppModel; let close: () -> Void; let managing: Bool
    @State private var name = ""; @State private var username = ""; @State private var error: String?; @State private var pending = false
    @State private var confirmDelete = false
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: managing ? "Manage space" : "Create a space", detail: managing ? "Only the owner can change this space and its membership." : nil, close: close)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    CaperField(title: "Space name", text: $name)
                    Button(managing ? "Save name" : "Create space") { run { if managing { try await model.renameSpace(name) } else { try await model.createSpace(name: name); close() } } }.buttonStyle(CaperPrimaryButton()).disabled(pending)
                    if managing {
                        Divider().overlay(CaperTheme.border)
                        Text("Members  \(model.detail?.members.count ?? 0)").font(CaperTheme.font(14, weight: .bold))
                            .accessibilityLabel("Members \(model.detail?.members.count ?? 0)")
                            .accessibilityIdentifier("space-members-heading")
                        HStack { TextField("Exact username", text: $username).textFieldStyle(CaperTextFieldStyle()); Button("Add") { run { try await model.addSpaceMember(username: username); username = "" } }.buttonStyle(.bordered) }
                        ForEach(model.detail?.members ?? []) { member in
                            HStack { Avatar(name: member.displayName, size: 30); VStack(alignment: .leading) { Text(member.displayName); Text("@\(member.username)\(member.owner ? " · Owner" : "")").foregroundStyle(CaperTheme.muted) }; Spacer(); if !member.owner { Button("Remove") { run { try await model.removeSpaceMember(member) } } } }.font(CaperTheme.font(12))
                        }
                        Divider().overlay(CaperTheme.border)
                        Text("Delete space").font(CaperTheme.font(14, weight: .bold))
                        Text("Delete this space and all its channels for every member.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                        Button("Delete space", role: .destructive) { CaperEffects.shared.play(.warning); confirmDelete = true }.buttonStyle(.bordered).disabled(pending)
                    }
                    if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)) }
                }.padding(22)
            }
        }.background(CaperTheme.surface).onAppear { name = managing ? model.detail?.space.name ?? "" : "" }
        .sheet(isPresented: $confirmDelete) {
            ConfirmationSheet(title: "Delete space", detail: "Delete \(model.detail?.space.name ?? name) for everyone? All its channels and their messages will disappear from the space. This cannot be undone.", action: "Delete space", close: { confirmDelete = false }) {
                try await model.deleteCurrentSpace()
                CaperEffects.shared.play(.delete)
                close()
            }
        }
    }
    private func run(_ action: @escaping () async throws -> Void) { pending = true; error = nil; Task { do { try await action() } catch { self.error = error.localizedDescription }; pending = false } }
}

private struct ChannelEditor: View {
    @Bindable var model: AppModel; @State var channel: Channel?; let close: () -> Void
    @State private var name = ""; @State private var privateChannel = false; @State private var members: [Member] = []; @State private var username = ""; @State private var error: String?; @State private var pending = false
    @State private var confirmDelete = false
    @State private var membersError: String?
    @State private var loadingMembers = false
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: channel == nil ? "Create a channel" : "Channel Overview", close: close)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    CaperField(title: "Channel name", text: Binding(get: { name }, set: { name = WorkspaceValidation.normalizeChannelName($0) }))
                    Toggle(isOn: Binding(get: { privateChannel }, set: { privateChannel = $0; CaperEffects.shared.toggle($0) })) { VStack(alignment: .leading) { Text("Private channel").font(CaperTheme.font(13, weight: .bold)); Text(privateChannel ? "Only you and the people you add can view or join." : "Anyone in this space can view or join this channel.").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted) } }.toggleStyle(.switch)
                    Button(channel == nil ? "Create channel" : "Save changes") { run { if let existing = channel { channel = try await model.updateChannel(existing, name: name, privateChannel: privateChannel) } else { try await model.createChannel(name: name, privateChannel: privateChannel); close() } } }.buttonStyle(CaperPrimaryButton()).disabled(pending)
                    if let channel, channel.private {
                        Divider().overlay(CaperTheme.border); Text("Members  \(members.count)").font(CaperTheme.font(14, weight: .bold))
                        if loadingMembers { ProgressView("Loading members…") }
                        if let membersError {
                            Text(membersError).foregroundStyle(.red)
                            Button("Retry loading members") { Task { await loadMembers(channel) } }.disabled(loadingMembers)
                        }
                        HStack { TextField("Exact username", text: $username).textFieldStyle(CaperTextFieldStyle()); Button("Add") { run { let member = try await model.addChannelMember(channel, username: username); members.removeAll { $0.id == member.id }; members.append(member); username = "" } }.buttonStyle(.bordered) }
                            .disabled(pending || loadingMembers || membersError != nil)
                        ForEach(members) { member in HStack { Avatar(name: member.displayName, size: 30); Text(member.displayName); Spacer(); if !member.owner { Button("Remove") { run { try await model.removeChannelMember(channel, member: member); members.removeAll { $0.id == member.id } } } } }.font(CaperTheme.font(12)) }
                            .disabled(pending || loadingMembers || membersError != nil)
                    }
                    if channel != nil { Divider().overlay(CaperTheme.border); Button("Delete channel", role: .destructive) { CaperEffects.shared.play(.warning); confirmDelete = true }.buttonStyle(.bordered).disabled(pending) }
                    if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)) }
                }.padding(22)
            }
        }.background(CaperTheme.surface).onAppear { name = channel?.name ?? ""; privateChannel = channel?.private ?? false }
        .task(id: channel?.private) { if let channel, channel.private { await loadMembers(channel) } }
        .sheet(isPresented: $confirmDelete) {
            if let channel {
                ConfirmationSheet(title: "Delete channel", detail: "Delete #\(channel.name) for everyone? This channel and its messages will disappear from the space. This cannot be undone.", action: "Delete channel", close: { confirmDelete = false }) {
                    try await model.deleteChannel(channel)
                    CaperEffects.shared.play(.delete)
                    close()
                }
            }
        }
    }
    private func loadMembers(_ channel: Channel) async {
        guard !loadingMembers else { return }
        loadingMembers = true; membersError = nil
        defer { loadingMembers = false }
        do { members = try await model.channelMembers(channel) }
        catch { membersError = error.localizedDescription }
    }
    private func run(_ action: @escaping () async throws -> Void) { pending = true; error = nil; Task { do { try await action() } catch { self.error = error.localizedDescription }; pending = false } }
}

private struct ConfirmationSheet: View {
    let title: String; let detail: String; let action: String; let close: () -> Void; let perform: () async throws -> Void
    @State private var pending = false; @State private var error: String?
    var body: some View { VStack(spacing: 0) { SheetHeader(title: title, detail: detail, close: { if !pending { close() } }); VStack(spacing: 16) { if let error { Text(error).foregroundStyle(.red) }; HStack { Button("Cancel", action: close).disabled(pending).keyboardShortcut(.cancelAction); Button(pending ? "Working…" : action, role: .destructive) { guard !pending else { return }; pending = true; error = nil; Task { do { try await perform(); close() } catch { self.error = error.localizedDescription }; pending = false } }.disabled(pending).accessibilityIdentifier("confirm-destructive-action") } }.padding(22) }.background(CaperTheme.surface).interactiveDismissDisabled(pending) }
}

private struct CaperField: View {
    let title: String; @Binding var text: String
    var body: some View { VStack(alignment: .leading, spacing: 7) { Text(title).font(CaperTheme.font(12, weight: .bold)); TextField(title, text: $text).textFieldStyle(CaperTextFieldStyle()) } }
}

private struct CaperTextFieldStyle: TextFieldStyle {
    func _body(configuration: TextField<Self._Label>) -> some View { configuration.font(CaperTheme.font(14)).padding(.horizontal, 11).frame(height: 42).background(CaperTheme.composer).overlay(RoundedRectangle(cornerRadius: 7).stroke(CaperTheme.border)).clipShape(RoundedRectangle(cornerRadius: 7)) }
}

private struct CaperPrimaryButton: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled
    func makeBody(configuration: Configuration) -> some View { configuration.label.font(CaperTheme.font(13, weight: .bold)).foregroundStyle(.white).frame(maxWidth: .infinity).frame(height: 42).background(configuration.isPressed ? CaperTheme.terracottaBright : CaperTheme.terracotta).clipShape(RoundedRectangle(cornerRadius: 8)).opacity(isEnabled ? 1 : 0.45) }
}

private struct AudioPreferencesView: View {
    @Bindable var voice: VoiceClient
    var debugEnabled = false
    @State private var controlsHeight: CGFloat = 500
    #if os(macOS)
    @Bindable private var effects = CaperEffects.shared
    @State private var micTest = MacMicrophoneTest()
    @State private var routeError: String?
    #else
    @State private var micTest = IOSMicrophoneTest()
    #endif
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                Text("Audio preferences").font(CaperTheme.font(20, weight: .bold))
                    .accessibilityIdentifier("audio-preferences-sheet")
                Spacer()
                Button { voice.showAudioPreferences = false } label: {
                    CaperIcon(name: "x").frame(width: 28, height: 28)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close audio preferences")
                .accessibilityIdentifier("close-audio-preferences")
                .keyboardShortcut(.cancelAction)
            }
            ScrollView {
                controls.fixedSize(horizontal: false, vertical: true)
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { controlsHeight = $0 }
            }
            .accessibilityIdentifier("audio-preferences-controls")
            #if os(macOS)
            .frame(width: 426, height: min(controlsHeight, 500))
            #else
            .frame(height: min(controlsHeight, 580))
            #endif
        }.padding(22)
            .frame(minWidth: 360)
            .background(CaperTheme.surface)
            #if os(iOS)
            .presentationDetents([.height(min(controlsHeight, 580) + 90)])
            #endif
            .task { await voice.refreshAudioDevices() }
            .onChange(of: voice.phase) { _, phase in
                if phase != .idle && phase != .failed { micTest.close() }
            }
            .onDisappear { micTest.close() }
            .task(id: voice.phase) {
                while !Task.isCancelled && voice.phase == .connected {
                    await voice.refreshDiagnostics()
                    do { try await Task.sleep(for: .seconds(2)) } catch { return }
                }
            }
    }

    private var controls: some View {
        VStack(alignment: .leading, spacing: 18) {
            #if os(macOS)
            Toggle("Caper sound effects", isOn: $effects.soundsEnabled)
                .toggleStyle(.checkbox)
                .accessibilityIdentifier("sound-effects")
            Picker("Input", selection: inputRoute) {
                Text("System default").tag("")
                ForEach(voice.availableInputs) { route in Text(route.name).tag(route.id) }
            }.accessibilityIdentifier("audio-input-device")
            Picker("Output", selection: outputRoute) {
                Text("System default").tag("")
                ForEach(voice.availableOutputs) { route in Text(route.name).tag(route.id) }
            }.accessibilityIdentifier("audio-output-device")
            if let routeError { Text(routeError).font(CaperTheme.font(11)).foregroundStyle(.red) }
            #else
            AudioRouteRow(title: "Input", value: voice.availableInputs.first(where: { $0.id == voice.selectedInputID })?.name ?? "System default")
            HStack {
                AudioRouteRow(title: "Output", value: voice.availableOutputs.first(where: { $0.id == voice.selectedOutputID })?.name ?? "System default")
                SystemAudioRoutePicker().frame(width: 44, height: 36)
                    .accessibilityLabel("Choose system audio route")
                    .accessibilityIdentifier("system-audio-route-picker")
            }
            #endif
            VStack(alignment: .leading, spacing: 7) {
                HStack { Text("Input volume"); Spacer(); Text("\(voice.inputGain)%") }.font(CaperTheme.font(12))
                Slider(value: inputGain, in: 0...200, step: 1)
                    .accessibilityLabel("Input volume")
                    .accessibilityValue("\(voice.inputGain)%")
            }
            VStack(alignment: .leading, spacing: 7) {
                HStack { Text("Voice processing"); Spacer(); Text("\(voice.voiceProcessingStrength)%") }.font(CaperTheme.font(12))
                Slider(value: liveStrength, in: 0...100, step: 1)
                    .accessibilityLabel("Voice processing")
                    .accessibilityValue("\(voice.voiceProcessingStrength)%")
            }
            VStack(alignment: .leading, spacing: 7) {
                HStack { Text("Output volume").font(CaperTheme.font(13, weight: .bold)); Spacer(); Text("\(voice.outputGain)%").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted) }
                Slider(value: outputGain, in: 0...200, step: 1)
                    .accessibilityLabel("Output volume")
                    .accessibilityValue("\(voice.outputGain)%")
            }
            Divider().overlay(CaperTheme.border)
            Text("Microphone test").font(CaperTheme.font(14, weight: .bold))
            HStack {
                Button(micTest.recording ? "Stop Testing" : "Mic Test") {
                    if micTest.recording { micTest.stopRecording() }
                    else { Task { await micTest.start(voice: voice) } }
                }
                .accessibilityIdentifier("local-mic-test")
                .disabled(recordedPreview || (voice.phase != .idle && voice.phase != .failed && voice.phase != .connected && !micTest.recording))
                if micTest.recording { Text("Recording your voice").font(CaperTheme.font(11)) }
            }
            if recordedPreview { Text("TEST FIXTURE — completed local recording layout only; no microphone or playback.")
                .font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.terracottaBright) }
            if micTest.hasRecording || recordedPreview {
                HStack {
                    Button("Play natural") { micTest.play(enhanced: false) }.disabled(recordedPreview)
                    Button("Play enhanced") { micTest.play(enhanced: true) }.disabled(recordedPreview)
                    Button("Stop playback") { micTest.stopPlayback() }.disabled(recordedPreview)
                }
            }
            if let error = micTest.error { Text(error).font(CaperTheme.font(11)).foregroundStyle(.red) }
            if debugEnabled {
                DisclosureGroup("Audio diagnostics") { AudioDiagnosticsView(voice: voice) }
            }
            if voice.phase == .connected || statisticsPreview {
                Divider().overlay(CaperTheme.border)
                Text("Connection statistics").font(CaperTheme.font(14, weight: .bold))
                if statisticsPreview { Text("TEST FIXTURE — synthetic statistics layout; no voice connection.")
                    .font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.terracottaBright) }
                if let stats = statisticsPreview ? previewStatistics : voice.diagnostics {
                    AudioRouteRow(title: "Receive / send", value: "\(stats.receiveBitrate.map { String($0) } ?? "—") / \(stats.sendBitrate.map { String($0) } ?? "—") bps")
                    AudioRouteRow(title: "Packets lost / max jitter", value: "\(stats.packetsLost) / \(stats.maxJitterMs.map { String($0) } ?? "—") ms")
                    AudioRouteRow(title: "RTT / route", value: "\(stats.roundTripMs.map { String($0) } ?? "—") ms / \(stats.route == "relay" ? "TURN relay" : stats.route == "direct" ? "Direct" : "Not observed yet")")
                } else {
                    Text("Waiting for transport statistics…").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                }
            }
        }
    }

    private var outputGain: Binding<Double> {
        Binding(get: { Double(voice.outputGain) }, set: { voice.setOutputGain(Int($0)); CaperEffects.shared.slider($0 / 200) })
    }
    #if os(macOS)
    private var inputRoute: Binding<String> {
        Binding(get: { voice.selectedInputID ?? "" }, set: { uid in
            routeError = voice.selectInput(uid) ? nil : "Could not switch microphone. The previous route is still selected."
        })
    }
    private var outputRoute: Binding<String> {
        Binding(get: { voice.selectedOutputID ?? "" }, set: { uid in
            routeError = voice.selectOutput(uid) ? nil : "Could not switch output. The previous route is still selected."
        })
    }
    #endif
    private var inputGain: Binding<Double> {
        Binding(get: { Double(voice.inputGain) }, set: { voice.setInputGain(Int($0)); CaperEffects.shared.slider($0 / 200) })
    }
    private var liveStrength: Binding<Double> {
        Binding(get: { Double(voice.voiceProcessingStrength) }, set: { voice.setVoiceProcessingStrength(Int($0)); CaperEffects.shared.slider($0 / 100) })
    }
    private var recordedPreview: Bool { CaperRuntime.isAudioPreview("audio-recorded") }
    private var statisticsPreview: Bool { CaperRuntime.isAudioPreview("audio-statistics") }
    private var previewStatistics: VoiceDiagnostics {
        VoiceDiagnostics(receivedBytes: 65_432, sentBytes: 12_345,
                         receiveBitrate: 12_800, sendBitrate: 24_000,
                         packetsLost: 3, maxJitterMs: 17, roundTripMs: 42, route: "relay")
    }
}

private struct AudioDiagnosticsView: View {
    let voice: VoiceClient
    @State private var copyStatus = ""

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { _ in
            let report = reportJSON
            VStack(alignment: .leading, spacing: 10) {
                Text("Local diagnostics only. No audio, device identifiers, or credentials. Nothing is uploaded.")
                    .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                Text(voice.noiseSuppressionStatus).font(CaperTheme.font(12))
                HStack {
                    Button("Copy diagnostics") {
                        #if os(macOS)
                        NSPasteboard.general.clearContents()
                        copyStatus = NSPasteboard.general.setString(report, forType: .string)
                            ? "Copied diagnostics" : "Copy failed; select the report below."
                        #else
                        UIPasteboard.general.string = report
                        copyStatus = "Copied diagnostics"
                        #endif
                    }
                    Text(copyStatus).font(CaperTheme.font(11))
                }
                Text(report).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
            }.accessibilityIdentifier("audio-diagnostics")
        }
    }

    private var reportJSON: String {
        #if os(macOS)
        let platform = "macOS"
        #else
        let platform = "iOS"
        #endif
        var values: [String: Any] = ["platform": platform, "processing": NSNull()]
        if let stats = voice.audioProcessingReport {
            values["processing"] = [
                "mode": stats.mode, "processedHops": stats.processedHops,
                "meanProcessingMs": stats.meanProcessingMs, "maxProcessingMs": stats.maxProcessingMs,
                "queuedInputMs": stats.queuedInputMs, "hopBudgetMs": 10,
            ] as [String: Any]
        }
        guard let data = try? JSONSerialization.data(withJSONObject: values, options: [.prettyPrinted, .sortedKeys]),
              let report = String(data: data, encoding: .utf8) else { return "Diagnostics unavailable." }
        return report
    }
}

private struct AudioRouteRow: View {
    let title: String
    let value: String
    var body: some View {
        HStack {
            Text(title).font(CaperTheme.font(13, weight: .bold))
            Spacer()
            Text(value).font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).lineLimit(1)
        }
    }
}

#if os(iOS)
private struct SystemAudioRoutePicker: UIViewRepresentable {
    func makeUIView(context: Context) -> MPVolumeView {
        let picker = MPVolumeView()
        picker.showsVolumeSlider = false
        picker.tintColor = UIColor(CaperTheme.text)
        picker.accessibilityLabel = "Choose system audio route"
        return picker
    }

    func updateUIView(_ view: MPVolumeView, context: Context) {}
}
#endif
