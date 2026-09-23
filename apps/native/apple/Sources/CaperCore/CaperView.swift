import SwiftUI
#if os(iOS)
import MediaPlayer
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
            if model.phase == .loading { await model.start() }
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
    @State private var sidebarWidth: CGFloat = 280
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
                HStack {
                    Wordmark()
                    Spacer()
                    if model.busy { ProgressView().controlSize(.small) }
                }
                .frame(maxWidth: 1400)
                .frame(height: narrow ? 64 : 80)
                .padding(.horizontal, narrow ? 12 : 28)

                Group {
                    if narrow && !model.navigationOpen {
                        ZStack(alignment: .trailing) {
                            ConversationStage(model: model, narrow: true, browse: { model.navigationOpen = true }, membersVisible: membersVisible) {
                                membersPreference = !membersVisible
                            }
                            if membersVisible { MemberPresenceView(model: model).frame(width: min(280, geometry.size.width - 24)) }
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
                            .frame(width: narrow ? nil : sidebarWidth)
                            .frame(maxWidth: narrow ? .infinity : sidebarWidth)
                            .overlay(alignment: .trailing) {
                                if !narrow {
                                    Rectangle().fill(Color.clear).frame(width: 8).contentShape(Rectangle())
                                        .gesture(DragGesture().onChanged { value in
                                            sidebarWidth = min(440, max(220, sidebarWidth + value.translation.width))
                                        })
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
                .frame(maxWidth: 1400, maxHeight: .infinity)
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(CaperTheme.border, lineWidth: 1))
                .padding(.horizontal, narrow ? 12 : 28)
                .padding(.bottom, 24)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(CaperTheme.blackout)
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
        }
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
                    .overlay(alignment: .leading) {
                        if model.selectedSpaceID == space.id {
                            RoundedRectangle(cornerRadius: 2).fill(CaperTheme.terracottaBright).frame(width: 3, height: 24).offset(x: -10)
                        }
                    }
                }
                Button(action: model.account == nil ? showLogin : create) {
                    Image(systemName: "plus").font(.system(size: 16, weight: .bold)).foregroundStyle(CaperTheme.terracottaBright)
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
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    HStack(spacing: 6) {
                        Menu {
                            if model.isOwner { Button("Space settings") { sheet = .manageSpace } }
                            else if model.account != nil && model.detail?.space.demo != true { Button("Leave space…", role: .destructive) { sheet = .leaveSpace } }
                        } label: {
                            HStack {
                                Text(model.detail?.space.name ?? "Caper").font(CaperTheme.font(15, weight: .bold)).lineLimit(1)
                                Spacer(); if model.detail?.space.demo != true { Image(systemName: "chevron.down").font(.system(size: 11, weight: .bold)) }
                            }.contentShape(Rectangle())
                        }.menuStyle(.borderlessButton).disabled(model.detail?.space.demo == true)
                            .accessibilityLabel(model.detail?.space.name ?? "Caper")
                            .accessibilityIdentifier("selected-space-name")
                        if narrow { Button(action: close) { Image(systemName: "xmark") }.buttonStyle(SidebarIconButton()) }
                    }
                    .frame(minHeight: 38).padding(.bottom, 12)
                    .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }

                    HStack {
                        Button { channelsExpanded.toggle() } label: {
                            HStack(spacing: 6) {
                                Image(systemName: "chevron.down").rotationEffect(.degrees(channelsExpanded ? 0 : -90))
                                Text("Channels")
                            }.font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                        }.buttonStyle(.plain)
                        Spacer()
                        if model.isOwner {
                            Button { sheet = .createChannel } label: { Image(systemName: "plus") }
                                .buttonStyle(SidebarIconButton()).disabled(!model.canCreateChannel)
                        }
                    }.frame(height: 44)

                    if channelsExpanded {
                        VStack(spacing: 3) {
                            ForEach(model.detail?.channels ?? []) { channel in
                                HStack(spacing: 2) {
                                    Button { Task { await model.select(channel: channel) } } label: {
                                        HStack(spacing: 9) {
                                            Image(systemName: channel.private ? "lock.fill" : "number")
                                                .font(.system(size: 15, weight: .semibold))
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
                                    if model.isOwner {
                                        Button { sheet = .manageChannel(channel) } label: { Image(systemName: "gearshape") }
                                            .buttonStyle(SidebarIconButton()).help("Manage \(channel.name)")
                                    }
                                }
                            }
                        }
                    }

                    if let error = model.error {
                        Text(error).font(CaperTheme.font(11)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)).padding(8)
                    }
                    if model.voice.phase == .connected || model.voice.phase == .reconnecting || !model.voice.participants.isEmpty {
                        VoiceRoster(model: model)
                    }
                }.padding(.horizontal, 16).padding(.top, 20)
            }
            AccountBar(model: model, sheet: $sheet)
        }
        .background(CaperTheme.sidebar)
        .overlay(alignment: .trailing) { Rectangle().fill(CaperTheme.border).frame(width: 1) }
    }
}

private struct SidebarIconButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.system(size: 14, weight: .semibold)).foregroundStyle(CaperTheme.muted)
            .frame(width: 28, height: 28).background(configuration.isPressed ? CaperTheme.border : .clear)
            .clipShape(RoundedRectangle(cornerRadius: 5))
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
            }.frame(minHeight: 38)
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
        }.padding(12).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(CaperTheme.sidebar)
            .overlay(alignment: .leading) { Rectangle().fill(CaperTheme.border).frame(width: 1) }
    }
    private func statusColor(_ status: PresenceStatus) -> Color {
        switch status { case .online: CaperTheme.green; case .idle: Color(red: 0.72, green: 0.60, blue: 0.35); case .offline, .unknown: CaperTheme.border }
    }
}

private struct VoiceRoster: View {
    @Bindable var model: AppModel
    @Bindable var voice: VoiceClient
    init(model: AppModel) { self.model = model; voice = model.voice }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Voice").font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                Spacer()
                switch voice.phase {
                case .idle, .failed: EmptyView()
                case .joining: Text("Joining…").font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
                case .connected: Text("Connected").font(CaperTheme.font(10)).foregroundStyle(CaperTheme.green)
                case .reconnecting: Text("Recovering…").font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
                case .leaving: ProgressView().controlSize(.small)
                }
            }
            ForEach(voice.participants) { participant in
                VStack(spacing: 7) {
                    HStack(spacing: 10) {
                        Avatar(name: participant.name, size: 38)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(participant.name).font(CaperTheme.font(14, weight: .bold)).lineLimit(1)
                            Text(participant.muted ? "Muted" : "Listening").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                        }
                        Spacer()
                        if !voice.isSelf(participantID: participant.id) {
                            Button {
                                voice.setParticipantMuted(!voice.locallyMutedParticipants.contains(participant.id), participantID: participant.id)
                            } label: {
                                Image(systemName: voice.locallyMutedParticipants.contains(participant.id) ? "speaker.slash.fill" : "speaker.wave.2.fill")
                            }.buttonStyle(SidebarIconButton()).accessibilityLabel("Mute \(participant.name) locally")
                        }
                        if participant.muted { Image(systemName: "mic.slash.fill").foregroundStyle(CaperTheme.muted) }
                        if participant.deafened { Image(systemName: "speaker.slash.fill").foregroundStyle(CaperTheme.muted) }
                    }
                    if !voice.isSelf(participantID: participant.id) {
                        HStack(spacing: 8) {
                            Slider(value: participantGain(participant.id), in: 0...200, step: 1)
                                .accessibilityLabel("\(participant.name) volume")
                            Text("\(voice.participantGains[participant.id] ?? 100)%")
                                .font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted).frame(width: 36, alignment: .trailing)
                        }
                    }
                }.padding(9).background(CaperTheme.raised.opacity(0.45)).clipShape(RoundedRectangle(cornerRadius: 8))
            }
            if let context = voice.context {
                HStack(spacing: 8) {
                    Button { Task { await model.openVoiceContext() } } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(voice.phase == .connected ? "Voice connected" : "Connecting voice…")
                                .font(CaperTheme.font(11, weight: .bold))
                            Text("\(context.spaceName) / \(context.channelName)")
                                .font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }.buttonStyle(.plain)
                    Button { voice.leaveImmediately() } label: { Image(systemName: "xmark") }
                        .buttonStyle(SidebarIconButton()).accessibilityLabel("Disconnect voice")
                }.padding(9).background(CaperTheme.raised).clipShape(RoundedRectangle(cornerRadius: 8))
            }
            if let error = voice.error { Text(error).font(CaperTheme.font(10)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)) }
        }.padding(.vertical, 16)
    }

    private func participantGain(_ id: String) -> Binding<Double> {
        Binding(
            get: { Double(voice.participantGains[id] ?? 100) },
            set: { voice.setParticipantGain(Int($0), participantID: id) }
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
    init(model: AppModel, sheet: Binding<WorkspaceSheet?>) { self.model = model; voice = model.voice; _sheet = sheet }
    var body: some View {
        HStack(spacing: 5) {
            Button { sheet = model.account == nil ? .login : .profile } label: {
                HStack(spacing: 7) {
                    Avatar(name: model.account?.displayName ?? "Guest", size: 30)
                    Text(model.account?.displayName ?? "Sign in").font(CaperTheme.font(13, weight: .medium)).lineLimit(1)
                    Spacer()
                }.contentShape(Rectangle())
            }.buttonStyle(.plain)
            if voice.phase == .connected || voice.phase == .reconnecting {
                Button { Task { await voice.setMuted(!voice.muted) } } label: { Image(systemName: voice.muted ? "mic.slash.fill" : "mic.fill") }.buttonStyle(SidebarIconButton())
                Button { Task { await voice.setDeafened(!voice.deafened) } } label: { Image(systemName: voice.deafened ? "speaker.slash.fill" : "headphones") }.buttonStyle(SidebarIconButton())
                Button(role: .destructive) { voice.leaveImmediately() } label: { Image(systemName: "phone.down.fill") }.buttonStyle(SidebarIconButton())
            }
            Menu {
                Button("Audio preferences") { voice.showAudioPreferences = true }
                if model.account != nil { Button("Log out", role: .destructive) { Task { await model.logout() } } }
            } label: { Image(systemName: "gearshape.fill") }.menuStyle(.borderlessButton).frame(width: 28)
                .accessibilityLabel("Account settings").accessibilityIdentifier("account-settings-menu")
        }
        .padding(4).frame(height: 42).background(CaperTheme.raised)
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(CaperTheme.border)).clipShape(RoundedRectangle(cornerRadius: 6))
        .padding(12)
        .sheet(isPresented: $voice.showAudioPreferences) { AudioPreferencesView(voice: voice).presentationBackground(CaperTheme.surface) }
    }
}

private struct Avatar: View {
    let name: String; let size: CGFloat
    var body: some View {
        Text(String(name.prefix(1)).uppercased()).font(CaperTheme.font(size * 0.36, weight: .black))
            .frame(width: size, height: size).background(Color(red: 53/255, green: 64/255, blue: 39/255)).clipShape(Circle())
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
                Image(systemName: "number").font(.system(size: 30)).foregroundStyle(CaperTheme.terracottaBright)
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
            HStack(spacing: 10) {
                if narrow {
                    Button(action: browse) { Label("Browse", systemImage: "line.3.horizontal") }.buttonStyle(.bordered).controlSize(.small)
                    VoiceHeaderButton(model: model, voice: voice)
                    Button(action: toggleMembers) { Image(systemName: "person.2.fill") }
                        .buttonStyle(SidebarIconButton()).accessibilityLabel(membersVisible ? "Hide members" : "Show members")
                }
                Text("# \(chat.channelName.lowercased())").font(CaperTheme.font(14, weight: .medium)).lineLimit(1)
                    .accessibilityLabel("# \(chat.channelName.lowercased())")
                    .accessibilityIdentifier("selected-channel-name")
                Spacer()
                if !narrow { VoiceHeaderButton(model: model, voice: voice) }
                if chat.liveState != .connected { Text(chat.liveState == .reconnecting ? "Reconnecting…" : "Connecting…").font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.muted) }
                if !narrow {
                    Button(action: toggleMembers) { Image(systemName: "person.2.fill") }
                        .buttonStyle(SidebarIconButton()).accessibilityLabel(membersVisible ? "Hide members" : "Show members")
                }
            }.padding(.horizontal, 18).frame(height: 50)
                .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }

            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 0) {
                        HStack {
                            if chat.hasMore { Button("Load older messages") { Task { await chat.loadOlder() } } }
                            else { Text("Beginning of conversation") }
                        }.font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.muted).frame(height: 44)
                        ForEach(chat.messages) { message in MessageRow(message: message).id(message.id) }
                        if let pending = chat.pendingMessage {
                            PendingMessageRow(pending: pending, author: chat.currentAuthor, error: chat.error) { Task { await chat.send() } }
                        }
                        if chat.messages.isEmpty && !chat.loading && chat.pendingMessage == nil {
                            VStack(spacing: 7) {
                                Text("No messages yet.").font(CaperTheme.font(14, weight: .medium))
                                Text("Start the conversation in #\(chat.channelName.lowercased()).").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                            }.padding(.top, 80)
                        }
                    }
                }
                .onChange(of: chat.messages.count) { _, _ in if let id = chat.messages.last?.id { proxy.scrollTo(id, anchor: .bottom) } }
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
                .disabled(chat.sending || chat.pendingMessage != nil || MessageValidation.error(for: chat.draft) != nil)
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
        if ProcessInfo.processInfo.environment["CAPER_EXPERIMENTAL_VOICE"] == "1" {
            if sameChannel, voice.phase == .joining || voice.phase == .reconnecting {
                Button("Cancel") { voice.leaveImmediately() }.buttonStyle(VoiceJoinButton())
            } else if sameChannel, voice.phase == .connected {
                Button { voice.leaveImmediately() } label: { Label("Leave", systemImage: "phone.down.fill") }.buttonStyle(VoiceJoinButton())
            } else if voice.phase == .leaving {
                ProgressView().controlSize(.small)
            } else {
                Button(action: joinSelectedChannel) { Label("Join", systemImage: "headphones") }
                    .buttonStyle(VoiceJoinButton()).disabled(selectedContext == nil)
            }
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
        guard let context = selectedContext else { return }
        if voice.phase != .idle && voice.phase != .failed { voice.leaveImmediately() }
        Task {
            await voice.join(
                channelID: model.detail?.space.demo == true ? nil : context.channelID,
                context: context,
                name: model.account?.displayName ?? "Guest"
            )
        }
    }
}

private struct PrimaryIconButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.foregroundStyle(.white).frame(width: 42, height: 42)
            .background(configuration.isPressed ? CaperTheme.terracottaBright : CaperTheme.terracotta)
            .clipShape(RoundedRectangle(cornerRadius: 8))
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
        guard let date = ISO8601DateFormatter().date(from: value) else { return "" }
        return date.formatted(date: .omitted, time: .shortened)
    }
}

private struct PendingMessageRow: View {
    let pending: PendingMessage; let author: ChatAuthor?; let error: String?; let retry: () -> Void
    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Avatar(name: author?.name ?? "Guest", size: 34)
            VStack(alignment: .leading, spacing: 4) {
                Text(author?.name ?? "Guest").font(CaperTheme.font(13, weight: .bold))
                Text(pending.text).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted)
                if let error {
                    HStack { Text("Not confirmed yet. \(error)"); Button("Retry send", action: retry) }
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
            CaperField(title: "Display name", text: $displayName)
            Button("Continue") { Task { await model.saveProfile(username: username, displayName: displayName) } }.buttonStyle(CaperPrimaryButton()).disabled(model.busy || username.isEmpty || displayName.isEmpty)
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
            Spacer(); Button(action: close) { Image(systemName: "xmark") }.buttonStyle(SidebarIconButton())
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
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(CaperTheme.font(16, weight: .medium)).foregroundStyle(.white)
            .padding(.horizontal, 20).frame(maxWidth: .infinity).frame(height: 58)
            .background(configuration.isPressed ? CaperTheme.terracottaBright : CaperTheme.terracotta)
            .clipShape(RoundedRectangle(cornerRadius: 6))
    }
}

private struct ProfileSheet: View {
    @Bindable var model: AppModel; let close: () -> Void
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: "Account", detail: model.account?.username.map { "@\($0)" }, close: close)
            VStack(alignment: .leading, spacing: 14) {
                HStack { Avatar(name: model.account?.displayName ?? "Caper", size: 42); Text(model.account?.displayName ?? "Caper").font(CaperTheme.font(16, weight: .bold)) }
                Button("Log out", role: .destructive) { Task { await model.logout(); close() } }.buttonStyle(.bordered)
            }.padding(22).frame(maxWidth: .infinity, alignment: .leading)
        }.background(CaperTheme.surface)
    }
}

private struct SpaceEditor: View {
    @Bindable var model: AppModel; let close: () -> Void; let managing: Bool
    @State private var name = ""; @State private var username = ""; @State private var error: String?; @State private var pending = false
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
                        Button("Delete space", role: .destructive) { run { try await model.deleteCurrentSpace(); close() } }.buttonStyle(.bordered)
                    }
                    if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)) }
                }.padding(22)
            }
        }.background(CaperTheme.surface).onAppear { name = managing ? model.detail?.space.name ?? "" : "" }
    }
    private func run(_ action: @escaping () async throws -> Void) { pending = true; error = nil; Task { do { try await action() } catch { self.error = error.localizedDescription }; pending = false } }
}

private struct ChannelEditor: View {
    @Bindable var model: AppModel; @State var channel: Channel?; let close: () -> Void
    @State private var name = ""; @State private var privateChannel = false; @State private var members: [Member] = []; @State private var username = ""; @State private var error: String?; @State private var pending = false
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: channel == nil ? "Create a channel" : "Channel Overview", close: close)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    CaperField(title: "Channel name", text: Binding(get: { name }, set: { name = WorkspaceValidation.normalizeChannelName($0) }))
                    Toggle(isOn: $privateChannel) { VStack(alignment: .leading) { Text("Private channel").font(CaperTheme.font(13, weight: .bold)); Text(privateChannel ? "Only you and the people you add can view or join." : "Anyone in this space can view or join this channel.").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted) } }.toggleStyle(.switch)
                    Button(channel == nil ? "Create channel" : "Save changes") { run { if let existing = channel { channel = try await model.updateChannel(existing, name: name, privateChannel: privateChannel) } else { try await model.createChannel(name: name, privateChannel: privateChannel); close() } } }.buttonStyle(CaperPrimaryButton()).disabled(pending)
                    if let channel, channel.private {
                        Divider().overlay(CaperTheme.border); Text("Members  \(members.count)").font(CaperTheme.font(14, weight: .bold))
                        HStack { TextField("Exact username", text: $username).textFieldStyle(CaperTextFieldStyle()); Button("Add") { run { let member = try await model.addChannelMember(channel, username: username); members.removeAll { $0.id == member.id }; members.append(member); username = "" } }.buttonStyle(.bordered) }
                        ForEach(members) { member in HStack { Avatar(name: member.displayName, size: 30); Text(member.displayName); Spacer(); if !member.owner { Button("Remove") { run { try await model.removeChannelMember(channel, member: member); members.removeAll { $0.id == member.id } } } } }.font(CaperTheme.font(12)) }
                    }
                    if let channel { Divider().overlay(CaperTheme.border); Button("Delete channel", role: .destructive) { run { try await model.deleteChannel(channel); close() } }.buttonStyle(.bordered) }
                    if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)) }
                }.padding(22)
            }
        }.background(CaperTheme.surface).onAppear { name = channel?.name ?? ""; privateChannel = channel?.private ?? false; if let channel, channel.private { Task { members = (try? await model.channelMembers(channel)) ?? [] } } }
    }
    private func run(_ action: @escaping () async throws -> Void) { pending = true; error = nil; Task { do { try await action() } catch { self.error = error.localizedDescription }; pending = false } }
}

private struct ConfirmationSheet: View {
    let title: String; let detail: String; let action: String; let close: () -> Void; let perform: () async throws -> Void
    @State private var pending = false; @State private var error: String?
    var body: some View { VStack(spacing: 0) { SheetHeader(title: title, detail: detail, close: close); VStack(spacing: 16) { if let error { Text(error).foregroundStyle(.red) }; HStack { Button("Cancel", action: close); Button(action, role: .destructive) { pending = true; Task { do { try await perform(); close() } catch { self.error = error.localizedDescription }; pending = false } }.disabled(pending) } }.padding(22) }.background(CaperTheme.surface) }
}

private struct CaperField: View {
    let title: String; @Binding var text: String
    var body: some View { VStack(alignment: .leading, spacing: 7) { Text(title).font(CaperTheme.font(12, weight: .bold)); TextField(title, text: $text).textFieldStyle(CaperTextFieldStyle()) } }
}

private struct CaperTextFieldStyle: TextFieldStyle {
    func _body(configuration: TextField<Self._Label>) -> some View { configuration.font(CaperTheme.font(14)).padding(.horizontal, 11).frame(height: 42).background(CaperTheme.composer).overlay(RoundedRectangle(cornerRadius: 7).stroke(CaperTheme.border)).clipShape(RoundedRectangle(cornerRadius: 7)) }
}

private struct CaperPrimaryButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View { configuration.label.font(CaperTheme.font(13, weight: .bold)).foregroundStyle(.white).frame(maxWidth: .infinity).frame(height: 42).background(configuration.isPressed ? CaperTheme.terracottaBright : CaperTheme.terracotta).clipShape(RoundedRectangle(cornerRadius: 8)) }
}

private struct AudioPreferencesView: View {
    @Bindable var voice: VoiceClient
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Audio preferences").font(CaperTheme.font(20, weight: .bold))
            AudioRouteRow(title: "Input", value: voice.availableInputs.first(where: { $0.id == voice.selectedInputID })?.name ?? "System default")
            AudioRouteRow(title: "Output", value: voice.availableOutputs.first(where: { $0.id == voice.selectedOutputID })?.name ?? "System default")
            VStack(alignment: .leading, spacing: 7) {
                HStack { Text("Output gain").font(CaperTheme.font(13, weight: .bold)); Spacer(); Text("\(voice.outputGain)%").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted) }
                Slider(value: outputGain, in: 0...200, step: 1).accessibilityLabel("Output gain")
            }
            #if os(iOS)
            HStack {
                Text("Choose an audio route").font(CaperTheme.font(13, weight: .bold))
                Spacer()
                SystemAudioRoutePicker().frame(width: 44, height: 36)
                    .accessibilityLabel("Choose system audio route")
                    .accessibilityIdentifier("system-audio-route-picker")
            }
            Text("Use the iPhone system picker to switch available routes during a call.")
                .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
            #else
            Text("Caper follows the input and output selected in macOS System Settings. The embedded WebRTC build does not expose safe per-device switching.")
                .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
            #endif
        }.padding(22).frame(minWidth: 360).background(CaperTheme.surface)
            .accessibilityIdentifier("audio-preferences-sheet")
            .task { await voice.refreshAudioDevices() }
    }

    private var outputGain: Binding<Double> {
        Binding(get: { Double(voice.outputGain) }, set: { voice.setOutputGain(Int($0)) })
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
