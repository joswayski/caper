import SwiftUI
#if os(iOS)
import MediaPlayer
import UIKit
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
            case .signedOut: LoginPage(model: model) {}
            case .onboarding: ProfileView(model: model)
            case .ready:
                if model.spaces.isEmpty && model.invitations.isEmpty && model.spacesLoaded { FirstSpaceView(model: model) }
                else if model.spaces.isEmpty, let error = model.spacesError { SpacesUnavailableView(model: model, error: error) }
                else { WorkspaceView(model: model) }
            }
        }
        .preferredColorScheme(.dark)
        .foregroundStyle(CaperTheme.text)
        .tint(CaperTheme.terracottaBright)
        .buttonStyle(CaperSecondaryButton())
        .background(CaperTheme.blackout.ignoresSafeArea())
        .task {
            CaperFontLoader.register()
            CaperEffects.shared.preload()
            if model.phase == .loading { await model.start() }
        }
        .task(id: model.voice.phase) {
            while !Task.isCancelled && model.voice.phase == .connected {
                await model.voice.sampleSpeaking()
                do { try await Task.sleep(for: .milliseconds(100)) } catch { break }
            }
            model.voice.clearSpeaking()
        }
        .onChange(of: model.voice.participants.map(\.id)) { old, new in
            // Web chimes when someone else joins or leaves the call you are in.
            guard model.voice.phase == .connected,
                  let me = old.first(where: { model.voice.isSelf(participantID: $0) }), new.contains(me) else { return }
            let before = Set(old).subtracting([me]), after = Set(new).subtracting([me])
            if !before.subtracting(after).isEmpty { CaperEffects.shared.play(.leave) }
            else if !after.subtracting(before).isEmpty { CaperEffects.shared.play(.join) }
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
            Text("Loading your spaces…").font(CaperTheme.font(12, weight: .medium)).foregroundStyle(CaperTheme.muted)
        }.frame(maxWidth: .infinity, maxHeight: .infinity).background(CaperTheme.blackout)
    }
}

/// Web's first-space page: an account with no spaces names one here.
private struct FirstSpaceView: View {
    @Bindable var model: AppModel
    @State private var name = ""
    @State private var error: String?
    @State private var pending = false
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Wordmark()
                Text("Name your space").font(CaperTheme.font(28, weight: .bold)).padding(.top, 20)
                    .fixedSize(horizontal: false, vertical: true)
                Text("Choose something you will recognize easily. You can always change it later!")
                    .font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted).fixedSize(horizontal: false, vertical: true)
                CaperField(title: "Space name", text: $name).onSubmit(create)
                Button(pending ? "Creating…" : "Create space", action: create)
                    .buttonStyle(CaperPrimaryButton())
                    .disabled(pending || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !model.canCreateSpace)
                    .accessibilityIdentifier("first-space-create")
                if !model.canCreateSpace, model.limits != nil {
                    Text("You have reached your space limit.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                }
                if let error {
                    Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.terracottaBright)
                        .fixedSize(horizontal: false, vertical: true)
                }
                HStack {
                    Spacer()
                    Button("Log out") { Task { await model.logout() } }.buttonStyle(.plain)
                        .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                }
            }.padding(28).frame(maxWidth: 440).frame(maxWidth: .infinity)
        }.background(CaperTheme.blackout)
    }
    private func create() {
        guard !pending, model.canCreateSpace, !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        pending = true; error = nil
        Task {
            // The name stays in the field when creation fails, like web.
            do { try await model.createSpace(name: name) } catch { self.error = error.localizedDescription }
            pending = false
        }
    }
}

private struct SpacesUnavailableView: View {
    @Bindable var model: AppModel
    let error: String
    var body: some View {
        VStack(spacing: 14) {
            Wordmark()
            Text("Spaces are unavailable.").font(CaperTheme.font(22, weight: .bold))
            Text(error).font(CaperTheme.font(13)).foregroundStyle(CaperTheme.terracottaBright).multilineTextAlignment(.center)
            Button(model.busy ? "Trying…" : "Try again") { Task { await model.loadSpaces() } }
                .buttonStyle(CaperSecondaryButton()).disabled(model.busy)
            Button("Log out") { Task { await model.logout() } }.buttonStyle(.plain)
                .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
        }.padding(28).frame(maxWidth: .infinity, maxHeight: .infinity).background(CaperTheme.blackout)
    }
}

private struct Wordmark: View {
    var body: some View {
        Image("CaperWordmark")
            .resizable()
            .scaledToFit()
            .frame(width: 132, height: 35)
            .accessibilityLabel("Caper")
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
    case login, profile, createSpace, createChannel, manageSpace, manageChannel(Channel), invitation(Space), channelInvitation(ChannelInvitation), leaveSpace, audio, connection, diagnostics
    var id: String {
        switch self {
        case .login: "login"
        case .profile: "profile"
        case .createSpace: "create-space"
        case .createChannel: "create-channel"
        case .manageSpace: "manage-space"
        case .manageChannel(let channel): "manage-\(channel.id)"
        case .invitation(let space): "invitation-\(space.id)"
        case .channelInvitation(let invitation): "channel-invitation-\(invitation.id)"
        case .leaveSpace: "leave-space"
        case .audio: "audio"
        case .connection: "connection"
        case .diagnostics: "diagnostics"
        }
    }
}

private struct WorkspaceView: View {
    @Bindable var model: AppModel
    @State private var sheet: WorkspaceSheet?
    @AppStorage("caper.channelSidebarWidth") private var sidebarWidth = 280.0
    @State private var sidebarDragStart: Double?
    @State private var sidebarHovered = false
    @FocusState private var sidebarFocused: Bool
    @State private var membersPreference: Bool?
    @State private var modalDismissDisabled = false
    private let parityFixture: String?

    init(model: AppModel) {
        self.model = model
        let environment = ProcessInfo.processInfo.environment
        parityFixture = environment["CAPER_TEST_MODE"] == "parity" ? environment["CAPER_UI_FIXTURE"] : nil
        _sheet = State(initialValue: parityFixture == "login" ? .login : nil)
    }

    var body: some View {
        workspace.task(id: model.viewedVoiceRoot) { await model.refreshVoiceAvailability() }
    }

    private var workspaceContent: some View {
        GeometryReader { geometry in
            let narrow = geometry.size.width <= 760
            let membersVisible = membersPreference ?? !narrow
            VStack(spacing: 0) {
                if let error = model.navigationError {
                    HStack {
                        Text(error).font(CaperTheme.font(12)).foregroundStyle(.red)
                        Spacer()
                        Button("Retry opening") { Task { await model.retryNavigation() } }
                            .disabled(model.openingSpaceID != nil)
                    }.padding(12).background(CaperTheme.surface)
                }
                Group {
                    if narrow && !model.navigationOpen && !model.spaces.isEmpty {
                        ZStack(alignment: .trailing) {
                            ConversationStage(model: model, narrow: true, browse: { model.navigationOpen = true }, createChannel: { sheet = .createChannel }, membersVisible: membersVisible) {
                                membersPreference = !membersVisible
                            }
                            if membersVisible {
                                Color.black.opacity(0.25)
                                    .contentShape(Rectangle())
                                    .onTapGesture { membersPreference = false }
                                    .padding(.top, 50)
                                    .accessibilityHidden(true)
                                MemberPresenceView(model: model, close: { membersPreference = false })
                                    .frame(width: min(280, geometry.size.width - 64))
                                    .clipShape(RoundedRectangle(cornerRadius: 16))
                                    .padding(.top, 58).padding(.trailing, 8).padding(.bottom, 8)
                            }
                        }
                    } else {
                        HStack(spacing: 0) {
                            SpaceRail(model: model, narrow: narrow, showLogin: { sheet = .login }, create: { sheet = .createSpace }, openInvitation: { sheet = .invitation($0) })
                                .frame(width: 60)
                            ChannelSidebar(
                                model: model,
                                sheet: $sheet,
                                narrow: narrow,
                                close: { model.navigationOpen = false }
                            )
                            .frame(width: narrow ? nil : CGFloat(min(sidebarWidth, sidebarMaximum(for: geometry.size.width))))
                            .frame(maxWidth: narrow ? .infinity : CGFloat(min(sidebarWidth, sidebarMaximum(for: geometry.size.width))))
                            .clipShape(RoundedRectangle(cornerRadius: narrow ? 16 : 0))
                            .padding(.top, narrow ? 8 : 0)
                            .padding(.trailing, narrow ? 8 : 0)
                            .overlay(alignment: .trailing) {
                                if !narrow {
                                    Rectangle().fill(sidebarHovered || sidebarFocused ? CaperTheme.terracottaBright : Color.clear)
                                        .frame(width: 2).frame(width: 8).frame(maxHeight: .infinity)
                                        .contentShape(Rectangle())
                                        .onHover { hovering in
                                            sidebarHovered = hovering
                                            #if os(macOS)
                                            if hovering { NSCursor.resizeLeftRight.push() } else { NSCursor.pop() }
                                            #endif
                                        }
                                        .highPriorityGesture(DragGesture(minimumDistance: 1, coordinateSpace: .global).onChanged { value in
                                            sidebarFocused = true
                                            if sidebarDragStart == nil { sidebarDragStart = sidebarWidth }
                                            resizeSidebar((sidebarDragStart ?? sidebarWidth) + Double(value.translation.width), viewport: geometry.size.width)
                                        }.onEnded { _ in sidebarDragStart = nil })
                                        .simultaneousGesture(TapGesture(count: 2).onEnded {
                                            resizeSidebar(280, viewport: geometry.size.width)
                                            sidebarFocused = true
                                        })
                                        .onTapGesture { sidebarFocused = true }
                                        .focusable()
                                        .focusEffectDisabled()
                                        .focused($sidebarFocused)
                                        // A plain view is a generic group to AppKit accessibility,
                                        // which carries no value. Present the handle as a slider
                                        // so VoiceOver and tests read and adjust the width.
                                        .accessibilityRepresentation {
                                            Slider(value: Binding(
                                                get: { min(sidebarWidth, sidebarMaximum(for: geometry.size.width)) },
                                                set: { resizeSidebar($0, viewport: geometry.size.width) }
                                            ), in: 220...max(230, sidebarMaximum(for: geometry.size.width)), step: 10)
                                        }
                                        .accessibilityLabel("Channel sidebar width")
                                        .accessibilityIdentifier("channel-sidebar-resize")
                                        .accessibilityValue("\(Int(min(sidebarWidth, sidebarMaximum(for: geometry.size.width)))) pixels")
                                        .accessibilityHint("Drag to resize. Arrow keys adjust by 10 pixels; Home and End select the bounds. Double-click resets.")
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
                                    if geometry.size.width - 60 - min(sidebarWidth, sidebarMaximum(for: geometry.size.width)) >= 540 {
                                        HStack(spacing: 0) {
                                            ConversationStage(model: model, narrow: false, browse: { model.navigationOpen = true }, createChannel: { sheet = .createChannel }, membersVisible: membersVisible) {
                                                membersPreference = !membersVisible
                                            }
                                            if membersVisible { MemberPresenceView(model: model).frame(width: 220) }
                                        }
                                    } else {
                                        ZStack(alignment: .trailing) {
                                            ConversationStage(model: model, narrow: false, browse: { model.navigationOpen = true }, createChannel: { sheet = .createChannel }, membersVisible: membersVisible) {
                                                membersPreference = !membersVisible
                                            }
                                            if membersVisible { MemberPresenceView(model: model).frame(width: 220).padding(.top, 50) }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                if narrow && model.navigationOpen {
                    AccountBar(model: model, sheet: $sheet)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(CaperTheme.blackout)
        }
    }

    private var workspace: some View {
        ZStack {
            workspaceContent
                .disabled(modalSheet.wrappedValue != nil)
                .accessibilityHidden(modalSheet.wrappedValue != nil)
            // A sibling, not an overlay on disabled content: modal controls must
            // retain their own hit-testing, keyboard and accessibility environment.
            if let item = modalSheet.wrappedValue {
                GeometryReader { geometry in
                    ZStack {
                        Color.black.opacity(0.55).ignoresSafeArea()
                            .onTapGesture { if !modalDismissDisabled { sheet = nil } }
                            .accessibilityHidden(true)
                        WorkspaceSheetView(item: item, model: model) { sheet = nil }
                            .id(item.id)
                            .frame(width: min(item.id.contains("manage") ? 600 : 560, geometry.size.width - 32))
                            .frame(height: min(680, max(1, geometry.size.height - 48)), alignment: .top)
                            .background(CaperTheme.surface.onTapGesture {})
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                            .shadow(color: .black.opacity(0.3), radius: 24, y: 12)
                            .accessibilityAddTraits(.isModal)
                            .onPreferenceChange(DialogDismissDisabled.self) { modalDismissDisabled = $0 }
                            #if os(macOS)
                            .onExitCommand { if !modalDismissDisabled { sheet = nil } }
                            #endif
                    }.frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
        }
        .modifier(LoginPresentation(sheet: $sheet, model: model))
        .onChange(of: sheet?.id) { _, _ in modalDismissDisabled = false }
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
    let narrow: Bool
    let showLogin: () -> Void
    let create: () -> Void
    let openInvitation: (Space) -> Void
    private var createHelp: String {
        if model.account == nil { return "Sign in to create a space" }
        if model.canCreateSpace { return "Create space" }
        return "Space limit reached (\(model.limits?.ownedSpaces ?? 20) owned, \(model.limits?.totalSpaces ?? 100) total)"
    }
    var body: some View {
        ScrollView {
            VStack(spacing: 10) {
                ForEach(model.spaces) { space in
                    SpaceRailButton(model: model, space: space, narrow: narrow)
                }
                if !model.invitations.isEmpty {
                    Text("INVITES").font(CaperTheme.font(8, weight: .bold)).foregroundStyle(CaperTheme.muted)
                        .accessibilityLabel("Pending invitations")
                    ForEach(model.invitations) { invitation in
                        Button { openInvitation(invitation) } label: {
                            Text(String(invitation.name.prefix(1)).uppercased()).font(CaperTheme.font(13, weight: .black))
                                .frame(width: 40, height: 40).background(CaperTheme.surface)
                                .clipShape(RoundedRectangle(cornerRadius: 12))
                                .overlay(RoundedRectangle(cornerRadius: 12).stroke(CaperTheme.terracottaBright, style: StrokeStyle(lineWidth: 1, dash: [3])))
                        }.buttonStyle(.plain).help("Invitation to \(invitation.name)").accessibilityLabel("Invitation to \(invitation.name)")
                    }
                }
                Button(action: model.account == nil ? showLogin : create) {
                    CaperIcon(name: "plus", size: 20).foregroundStyle(CaperTheme.terracottaBright)
                        .frame(width: narrow ? 44 : 40, height: narrow ? 44 : 40)
                        .background(CaperTheme.surface)
                        .clipShape(RoundedRectangle(cornerRadius: 12))
                        .overlay(RoundedRectangle(cornerRadius: 12).stroke(CaperTheme.border, style: StrokeStyle(lineWidth: 1, dash: [4])))
                }
                .buttonStyle(.plain).disabled(model.account != nil && !model.canCreateSpace)
                .help(createHelp)
            }.padding(.vertical, 14).frame(maxWidth: .infinity)
        }
        .background(CaperTheme.blackout)
        .overlay(alignment: .trailing) { if !narrow { Rectangle().fill(CaperTheme.border).frame(width: 1) } }
    }
}

/// One space in the rail. Kept out of SpaceRail's body so the Swift type
/// checker handles each piece in reasonable time.
private struct SpaceRailButton: View {
    @Bindable var model: AppModel
    let space: Space
    let narrow: Bool
    private var selected: Bool { model.selectedSpaceID == space.id }
    private var name: String { space.demo == true ? "Caper" : space.name }
    private var corner: CGFloat { selected ? 8 : 12 }
    private var fill: Color { selected ? Color(red: 57/255, green: 35/255, blue: 30/255) : CaperTheme.surface }
    private var edge: Color { selected ? Color(red: 128/255, green: 81/255, blue: 67/255) : CaperTheme.border }
    private var state: String { model.openingSpaceID == space.id ? "Opening" : selected ? "Selected" : "" }

    var body: some View {
        Button { Task { await model.select(space: space) } } label: {
            Text(space.demo == true ? "C" : String(space.name.prefix(1)).uppercased())
                .font(CaperTheme.font(13, weight: .black))
                .frame(width: narrow ? 44 : 40, height: narrow ? 44 : 40)
                .background(fill)
                .clipShape(RoundedRectangle(cornerRadius: corner))
                .overlay(RoundedRectangle(cornerRadius: corner).stroke(edge))
        }
        .buttonStyle(.plain).help(name)
        .modifier(NavigationPrefetchModifier { model.prefetch(space: space) })
        .accessibilityLabel(name)
        .accessibilityValue(state)
        .overlay(alignment: .leading) {
            if selected {
                RoundedRectangle(cornerRadius: 2).fill(CaperTheme.terracottaBright).frame(width: 3, height: 24).offset(x: -10)
            }
        }
    }
}

private struct ChannelSidebar: View {
    @Bindable var model: AppModel
    @Binding var sheet: WorkspaceSheet?
    let narrow: Bool
    let close: () -> Void
    @State private var channelsExpanded = true
    @State private var channelSearch = ""
    @State private var browsing = false
    var body: some View {
        VStack(spacing: 0) {
                    HStack(spacing: 6) {
                        Menu {
                            if model.isOwner { Button("Space settings") { sheet = .manageSpace } }
                            else if model.account != nil && model.detail?.space.demo != true { Button("Leave space…", role: .destructive) { sheet = .leaveSpace } }
                        } label: {
                            HStack {
                                Text(model.detail?.space.demo == true ? "Caper" : model.detail?.space.name ?? "Caper").font(CaperTheme.font(15, weight: .bold)).lineLimit(1)
                                Spacer(); if model.detail?.space.demo != true { CaperIcon(name: "chevron-down") }
                            }.contentShape(Rectangle())
                        }.menuStyle(.borderlessButton).menuIndicator(.hidden).disabled(model.detail?.space.demo == true)
                            .foregroundStyle(CaperTheme.text)
                            .accessibilityLabel(model.detail?.space.demo == true ? "Caper" : model.detail?.space.name ?? "Caper")
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
                                Text("\(model.detail?.channels.filter(\.joined).count ?? 0)").font(CaperTheme.font(10, weight: .bold))
                            }.font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                        }.buttonStyle(.plain)
                        Spacer()
                        if model.isOwner {
                            let createHelp = model.canCreateChannel ? "Create channel" : "Channel limit reached (\(model.limits?.channelsPerSpace ?? 100))"
                            Button { sheet = .createChannel } label: { CaperIcon(name: "plus") }
                                .buttonStyle(SidebarIconButton()).disabled(!model.canCreateChannel).help(createHelp).accessibilityLabel("Create channel")
                            // Web's owner-only Channel options menu.
                            Menu {
                                Button("Create channel") { sheet = .createChannel }.disabled(!model.canCreateChannel)
                                Button("\(channelsExpanded ? "Collapse" : "Expand") channels") { channelsExpanded.toggle() }
                            } label: { CaperIcon(name: "ellipsis") }
                                .menuStyle(.borderlessButton).menuIndicator(.hidden).frame(width: 28, height: 28)
                                .help("Channel options").accessibilityLabel("Channel options")
                        }
                    }.frame(height: 44)

                    if channelsExpanded {
                        VStack(spacing: 3) {
                            ForEach(model.detail?.channels.filter(\.joined) ?? []) { channel in
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
                                    if model.voice.context?.channelID != channel.id || model.voice.phase == .idle || model.voice.phase == .failed {
                                        if model.selectedChannelID == channel.id || !model.voicePresence.roster(for: channel.id).isEmpty {
                                            Button("Join") { Task { await model.joinVoice(channel: channel) } }
                                                .buttonStyle(VoiceJoinButton()).disabled(model.voiceAvailable != true)
                                                .fixedSize()
                                                .help(voiceAvailabilityHelp(model) ?? "Join voice in #\(channel.name)")
                                                .accessibilityLabel(model.voice.phase == .idle || model.voice.phase == .failed
                                                    ? "Join voice in #\(channel.name)" : "Switch voice to #\(channel.name)")
                                                .accessibilityIdentifier("join-voice-\(channel.id)")
                                                .modifier(PrepareVoiceOnApproach { model.prepareVoiceJoin(channel: channel) })
                                        }
                                    }
                                    if model.isOwner {
                                        Button { sheet = .manageChannel(channel) } label: { CaperIcon(name: "settings") }
                                            .buttonStyle(SidebarIconButton()).help("Manage \(channel.name)").accessibilityLabel("Manage \(channel.name)")
                                    }
                                }
                                ChannelVoiceSlot(model: model, channel: channel)
                            }
                        }
                    }

                    Divider().overlay(CaperTheme.border).padding(.vertical, 10)
                    Button(browsing ? "Close Browse" : "Browse channels") { browsing.toggle(); channelSearch = "" }
                        .buttonStyle(.plain).font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                    if browsing {
                    TextField("Search channels", text: $channelSearch).textFieldStyle(CaperTextFieldStyle()).padding(.vertical, 6)
                    ForEach((model.detail?.channels ?? []).filter { channelSearch.isEmpty || $0.name.localizedCaseInsensitiveContains(channelSearch) }) { channel in
                        Button { Task { await model.select(channel: channel) } } label: {
                            HStack { CaperIcon(name: channel.private ? "lock" : "hash", size: 16); Text(channel.name); Spacer(); Text(channel.joined ? "Joined" : "Preview").font(CaperTheme.font(10)) }
                        }.buttonStyle(.plain).padding(.vertical, 6)
                    }
                    }
                    if let invitations = model.detail?.channelInvitations, !invitations.isEmpty {
                        Text("Private invitations").font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted).padding(.top, 12)
                        ForEach(invitations) { invitation in
                            Button("#\(invitation.channel.name) · from @\(invitation.inviter.username)") { sheet = .channelInvitation(invitation) }
                                .buttonStyle(.plain).padding(.vertical, 6)
                        }
                    }

                    if let error = model.error {
                        Text(error).font(CaperTheme.font(11)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)).padding(8)
                    }
                    if (model.voice.phase == .connected || model.voice.phase == .reconnecting || !model.voice.participants.isEmpty),
                       (!channelsExpanded || !((model.detail?.channels ?? []).contains { $0.id == model.voice.context?.channelID })) {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("In voice · \(model.voice.participants.count) · \(model.voice.context?.channelName ?? "general")")
                                .font(CaperTheme.font(10, weight: .bold)).foregroundStyle(CaperTheme.muted)
                                .accessibilityIdentifier("voice-elsewhere-label")
                            VoiceRoster(model: model)
                        }.padding(.top, 8)
                    }
                }.padding(.horizontal, 16)
            }
            if !narrow { AccountBar(model: model, sheet: $sheet) }
        }
        .background(CaperTheme.sidebar)
        .overlay(alignment: .trailing) { if !narrow { Rectangle().fill(CaperTheme.border).frame(width: 1) } }
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
            .modifier(ControlHover())
        #else
        content
        #endif
    }
}

private struct SidebarIconButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.system(size: 14, weight: .semibold)).foregroundStyle(CaperTheme.muted)
            #if os(iOS)
            .frame(width: 44, height: 44)
            #else
            .frame(width: 28, height: 28).background(configuration.isPressed ? CaperTheme.border : .clear)
            #endif
            .clipShape(RoundedRectangle(cornerRadius: 5))
            .modifier(ControlHover())
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
                VoiceSpectator(id: $0.id, avatarId: $0.avatarId, name: $0.name, muted: $0.muted,
                               deafened: $0.deafened)
            }
        }
        return model.voicePresence.roster(for: channel.id)
    }

    var body: some View {
        let occupants = people
        if !occupants.isEmpty {
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 8) {
                    if !occupants.isEmpty {
                        Button { collapsed.toggle() } label: {
                            HStack(spacing: 5) {
                                ForEach(occupants.prefix(3)) { person in
                                    Avatar(name: person.name, size: 20, avatarID: person.avatarId, speaking: active && model.voice.speakingParticipants.contains(person.id))
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
                }
                if active {
                    if !collapsed { VoiceRoster(model: model) }
                } else if !collapsed {
                    ForEach(occupants) { person in
                        HStack(spacing: 7) {
                            Avatar(name: person.name, size: 23, avatarID: person.avatarId)
                            Text(person.name).lineLimit(1)
                            if person.muted { CaperIcon(name: "mic-off", size: 13) }
                            if person.deafened { CaperIcon(name: "headphone-off", size: 13) }
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
    var close: (() -> Void)?
    init(model: AppModel, close: (() -> Void)? = nil) { self.model = model; presence = model.presence; self.close = close }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Members").font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                Spacer()
                if model.detail?.space.demo != true {
                    Text("\(model.detail?.members.count ?? 0)").font(CaperTheme.font(10, weight: .bold)).foregroundStyle(CaperTheme.muted)
                }
                if let close {
                    Button(action: close) { CaperIcon(name: "x") }
                        .buttonStyle(SidebarIconButton()).accessibilityLabel("Close member list")
                }
            }.padding(.horizontal, 12).frame(height: 50)
                .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
            ScrollView {
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
                            Avatar(name: member.displayName, size: 30, avatarID: member.avatarId)
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
            }
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(CaperTheme.sidebar)
            .overlay(alignment: .leading) { Rectangle().fill(CaperTheme.border).frame(width: 1) }
    }
    private func statusColor(_ status: PresenceStatus) -> Color {
        switch status { case .online: CaperTheme.green; case .idle: Color(red: 0.72, green: 0.60, blue: 0.35); case .offline, .unknown: CaperTheme.border }
    }
}

#if os(macOS)
/// Web opens a participant's audio menu on right-click. A local event monitor
/// checks the row's frame, so nothing is layered over the row's own controls.
private final class RightClickBox {
    var frame: CGRect = .zero
    var monitor: Any?
}

private struct RightClickModifier: ViewModifier {
    let enabled: Bool
    let action: () -> Void
    @State private var box = RightClickBox()
    func body(content: Content) -> some View {
        content
            .background(GeometryReader { proxy in
                Color.clear
                    .onAppear { box.frame = proxy.frame(in: .global) }
                    .onChange(of: proxy.frame(in: .global)) { _, frame in box.frame = frame }
            })
            .onAppear {
                guard enabled, box.monitor == nil else { return }
                let box = box
                box.monitor = NSEvent.addLocalMonitorForEvents(matching: .rightMouseDown) { event in
                    guard let height = event.window?.contentView?.bounds.height else { return event }
                    let point = CGPoint(x: event.locationInWindow.x, y: height - event.locationInWindow.y)
                    guard box.frame.contains(point) else { return event }
                    action()
                    return nil
                }
            }
            .onDisappear {
                if let monitor = box.monitor { NSEvent.removeMonitor(monitor) }
                box.monitor = nil
            }
    }
}

private extension View {
    func onRightClick(enabled: Bool, perform action: @escaping () -> Void) -> some View {
        modifier(RightClickModifier(enabled: enabled, action: action))
    }
}
#endif

private struct VoiceRoster: View {
    @Bindable var voice: VoiceClient
    @State private var audioParticipantID: String? = nil
    init(model: AppModel) { voice = model.voice }
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(voice.participants) { participant in
                HStack(spacing: 8) {
                    Avatar(name: participant.name, size: 20, avatarID: participant.avatarId, speaking: voice.speakingParticipants.contains(participant.id))
                        .accessibilityValue(voice.speakingParticipants.contains(participant.id) ? "Speaking" : "")
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
                        CaperIcon(name: "headphone-off", size: 14).accessibilityHidden(false).accessibilityLabel("Deafened")
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
                                    CaperSlider(value: participantGain(participant.id), in: 0...200, step: 1)
                                        .accessibilityLabel("\(participant.name) volume")
                                    Toggle("Mute", isOn: participantMute(participant.id))
                                        .font(CaperTheme.font(12, weight: .medium))
                                    Text("Only changes what you hear.").font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
                                }.padding(16).frame(width: 220).background(CaperTheme.surface)
                                #if os(iOS)
                                .presentationCompactAdaptation(.popover)
                                #endif
                            }
                    }
                }.foregroundStyle(CaperTheme.muted).padding(.vertical, 4)
                #if os(macOS)
                .contentShape(Rectangle())
                .onRightClick(enabled: !voice.isSelf(participantID: participant.id)) { audioParticipantID = participant.id }
                #endif
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

/// Web prepares a join as the pointer nears Join, or on touch-down.
private struct PrepareVoiceOnApproach: ViewModifier {
    let prepare: () -> Void
    func body(content: Content) -> some View {
        #if os(macOS)
        // Hovering Join itself: a wider hover region would take clicks from neighboring rows.
        content.onHover { if $0 { prepare() } }
        #else
        content.simultaneousGesture(DragGesture(minimumDistance: 0).onChanged { _ in prepare() })
        #endif
    }
}

private struct VoiceJoinButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.terracottaBright)
            .padding(.horizontal, 10)
            #if os(iOS)
            .frame(height: 44)
            #else
            .frame(height: 32)
            #endif
            .background(CaperTheme.terracotta.opacity(configuration.isPressed ? 0.25 : 0.14))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(CaperTheme.terracotta.opacity(0.7))).clipShape(RoundedRectangle(cornerRadius: 6))
            .modifier(ControlHover())
    }
}

private struct AccountBar: View {
    @Bindable var model: AppModel
    @Bindable var voice: VoiceClient
    @Binding var sheet: WorkspaceSheet?
    @Bindable private var effects = CaperEffects.shared
    @State private var inputOptions = false
    @State private var outputOptions = false
    init(model: AppModel, sheet: Binding<WorkspaceSheet?>) { self.model = model; voice = model.voice; _sheet = sheet }
    /// Web's identityName: the account's display name, else the chat identity's.
    private var identityName: String { model.account?.displayName ?? model.chat.currentAuthor?.name ?? "Guest" }
    /// Your own status in the space's presence, else the live chat connection (web's localPresence).
    private var ownPresence: PresenceStatus? {
        if let id = model.account?.id, let status = model.presence.statuses[id], status != .unknown { return status }
        return model.chat.liveState == .connected ? .online : nil
    }
    var body: some View {
        VStack(spacing: 0) {
            if let context = voice.context, voice.phase != .idle && voice.phase != .failed {
                HStack(spacing: 8) {
                    Button { Task { await model.openVoiceContext() } } label: {
                        HStack(spacing: 8) {
                        // Web: green when connected, amber while connecting or reconnecting.
                        let tone = voice.phase == .connected ? Color(red: 140/255, green: 178/255, blue: 98/255)
                            : Color(red: 217/255, green: 171/255, blue: 92/255)
                        CaperIcon(name: "audio-lines", size: 16).foregroundStyle(tone)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(voice.phase == .connected ? "Voice connected" : voice.phase == .joining ? "Connecting…" : "Reconnecting…")
                                .font(CaperTheme.font(11, weight: .bold)).foregroundStyle(tone)
                            Text("\(context.channelName) / \(context.spaceName)")
                                .font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                        }
                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }.buttonStyle(.plain)
                    Button { if voice.phase == .connected { CaperEffects.shared.play(.disconnect) }; model.leaveVoice() } label: { CaperIcon(name: "phone-off") }
                        .buttonStyle(SidebarIconButton()).help(voice.phase == .connected ? "Disconnect" : "Cancel")
                        .accessibilityLabel(voice.phase == .connected ? "Leave voice" : "Cancel joining voice")
                }.padding(9).background(CaperTheme.raised).clipShape(RoundedRectangle(cornerRadius: 8))
                    .accessibilityIdentifier("active-voice-context")
            }
            if let error = voice.error {
                HStack(alignment: .top, spacing: 8) {
                    Text(error).font(CaperTheme.font(11)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51))
                        .fixedSize(horizontal: false, vertical: true).frame(maxWidth: .infinity, alignment: .leading)
                    Button { voice.error = nil } label: { CaperIcon(name: "x", size: 14) }
                        .buttonStyle(SidebarIconButton()).accessibilityLabel("Dismiss voice error")
                }.padding(9).background(CaperTheme.raised).clipShape(RoundedRectangle(cornerRadius: 8))
                    .accessibilityIdentifier("voice-error")
            }
            HStack(spacing: 5) {
            Button { sheet = model.account == nil ? .login : .profile } label: {
                HStack(spacing: 7) {
                    Avatar(name: identityName, size: 30, avatarID: model.account?.avatarId)
                        .overlay(alignment: .bottomTrailing) { PresenceDot(status: ownPresence, live: model.presence.online) }
                    Text(identityName).font(CaperTheme.font(13, weight: .medium)).lineLimit(1).truncationMode(.tail)
                }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
            }.buttonStyle(.plain)
                // Web: the account's name, else the guest chat identity, with its profile label.
                .accessibilityLabel(model.account == nil ? "Sign in to edit your profile" : "Edit profile for \(identityName)")
                .accessibilityIdentifier("account-profile")
            Button { CaperEffects.shared.toggle(voice.muted); Task { await voice.setMuted(!voice.muted) } } label: {
                CaperIcon(name: voice.muted ? "mic-off" : "mic", size: 20)
                    .foregroundStyle(voice.muted ? CaperTheme.terracottaBright : CaperTheme.muted)
            }.buttonStyle(SidebarIconButton()).help(voice.muted ? "Unmute" : "Mute")
                .accessibilityLabel(voice.muted ? "Unmute microphone" : "Mute microphone")
                .accessibilityValue(voice.muted ? "Muted" : "On").accessibilityIdentifier("microphone-toggle")
            Button { inputOptions.toggle() } label: {
                CaperIcon(name: "chevron-down", size: 12)
                    #if os(iOS)
                    .frame(width: 32, height: 44)
                    #else
                    .frame(width: 14, height: 28)
                    #endif
                    .contentShape(Rectangle())
            }.buttonStyle(.plain).help("Input Options").accessibilityLabel("Input Options")
                .popover(isPresented: $inputOptions, arrowEdge: .top) { AccountAudioMenu(voice: voice, input: true) }
            Button { CaperEffects.shared.toggle(voice.deafened); Task { await voice.setDeafened(!voice.deafened) } } label: {
                CaperIcon(name: voice.deafened ? "volume-x" : "headphones", size: 20)
                    .foregroundStyle(voice.deafened ? CaperTheme.terracottaBright : CaperTheme.muted)
            }.buttonStyle(SidebarIconButton()).help(voice.deafened ? "Undeafen" : "Deafen")
                .accessibilityLabel(voice.deafened ? "Undeafen audio" : "Deafen audio")
                .accessibilityValue(voice.deafened ? "Deafened" : "On").accessibilityIdentifier("deafen-toggle")
            Button { outputOptions.toggle() } label: {
                CaperIcon(name: "chevron-down", size: 12)
                    #if os(iOS)
                    .frame(width: 32, height: 44)
                    #else
                    .frame(width: 14, height: 28)
                    #endif
                    .contentShape(Rectangle())
            }.buttonStyle(.plain).help("Output Options").accessibilityLabel("Output Options")
                .popover(isPresented: $outputOptions, arrowEdge: .top) { AccountAudioMenu(voice: voice, input: false) }
            Menu {
                // Web's User Settings menu.
                Section("Audio settings") {
                    Toggle("Caper sound effects", isOn: $effects.soundsEnabled)
                        .accessibilityIdentifier("sound-effects")
                }
                Button("Audio test") { sheet = .audio }
                    .disabled(voice.phase == .leaving)
                if voice.phase == .connected || CaperRuntime.isAudioPreview("audio-statistics") {
                    Button("Connection details") { sheet = .connection }
                }
                #if os(macOS)
                if model.account?.debugEnabled == true {
                    Button("Audio diagnostics") { sheet = .diagnostics }
                }
                #endif
                if model.account != nil { Button("Log out", role: .destructive) { Task { await model.logout() } } }
                else { Button("Sign in") { sheet = .login } }
            } label: { CaperIcon(name: "settings", size: 20) }.menuStyle(.borderlessButton).menuIndicator(.hidden)
                #if os(iOS)
                .frame(width: 44, height: 44)
                #else
                .frame(width: 28, height: 28)
                #endif
                .fixedSize()
                .accessibilityLabel("Account settings").accessibilityIdentifier("account-settings-menu")
            }.padding(4)
                #if os(iOS)
                .frame(height: 52)
                #else
                .frame(height: 42)
                #endif
                .background(CaperTheme.raised)
        }
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(CaperTheme.border)).clipShape(RoundedRectangle(cornerRadius: 6))
        .padding(12)
    }
}

/// Web's Input Options / Output Options menus.
/// Web's PresenceDot: colored by status, labelled "Online", "Idle (last known;
/// reconnecting)" or "Status unavailable".
private struct PresenceDot: View {
    let status: PresenceStatus?
    var live = true
    var body: some View {
        let label = status.map { "\($0.rawValue.prefix(1).uppercased())\($0.rawValue.dropFirst())\(live ? "" : " (last known; reconnecting)")" } ?? "Status unavailable"
        Circle().fill(color).frame(width: 10, height: 10).overlay(Circle().stroke(CaperTheme.raised, lineWidth: 2))
            .help(label).accessibilityElement().accessibilityLabel(label)
    }
    private var color: Color {
        switch status { case .online?: CaperTheme.green; case .idle?: Color(red: 0.72, green: 0.60, blue: 0.35); default: CaperTheme.border }
    }
}

private struct AccountAudioMenu: View {
    @Bindable var voice: VoiceClient
    let input: Bool
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(input ? "Microphone" : "Audio output").font(CaperTheme.font(13, weight: .bold))
            #if os(macOS)
            CaperDevicePicker(title: "Device", selection: Binding(get: { (input ? voice.selectedInputID : voice.selectedOutputID) ?? "" }, set: { uid in
                let changed = input ? voice.selectInput(uid) : voice.selectOutput(uid)
                error = changed ? nil : "Could not switch devices. Check system audio settings."
            }), devices: input ? voice.availableInputs : voice.availableOutputs)
            #else
            // iPhone follows the system audio route.
            HStack {
                Text((input ? voice.availableInputs.first(where: { $0.id == voice.selectedInputID }) : voice.availableOutputs.first(where: { $0.id == voice.selectedOutputID }))?.name ?? "System default")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                Spacer()
                if !input {
                    SystemAudioRoutePicker().frame(width: 44, height: 36).accessibilityLabel("Choose system audio route")
                }
            }
            #endif
            if let error { Text(error).font(CaperTheme.font(11)).foregroundStyle(.red) }
            if input {
                Text("Input volume · \(voice.inputGain)%").font(CaperTheme.font(12))
                CaperSlider(value: Binding(get: { Double(voice.inputGain) }, set: { voice.setInputGain(Int($0)); CaperEffects.shared.slider($0 / 200) }), in: 0...200, step: 1)
                    .accessibilityLabel("Input volume")
            } else {
                Text("Output volume · \(voice.outputGain)%").font(CaperTheme.font(12))
                CaperSlider(value: Binding(get: { Double(voice.outputGain) }, set: { voice.setOutputGain(Int($0)); CaperEffects.shared.slider($0 / 200) }), in: 0...200, step: 1)
                    .accessibilityLabel("Output volume")
            }
        }.padding(16).frame(width: 260).background(CaperTheme.surface)
            #if os(iOS)
            .presentationCompactAdaptation(.popover)
            #endif
            .task { await voice.refreshAudioDevices() }
    }
}

struct Avatar: View {
    let name: String; let size: CGFloat
    var avatarID: Int? = nil
    var speaking = false
    var body: some View {
        Group {
            if let image = CaperAvatar.image(for: avatarID) {
                #if os(iOS)
                Image(uiImage: image).renderingMode(.original)
                    .resizable()
                    .scaledToFit()
                    .frame(width: size, height: size)
                #else
                Image(nsImage: image).renderingMode(.original)
                    .resizable()
                    .scaledToFit()
                    .frame(width: size, height: size)
                #endif
            } else {
                Text(String(name.prefix(1)).uppercased()).font(CaperTheme.font(size * 0.36, weight: .black))
                    .frame(width: size, height: size).background(CaperTheme.raised)
            }
        }.frame(width: size, height: size).clipShape(Circle())
            // Web: caper-green border with a soft outer ring while speaking.
            .overlay { if speaking { Circle().stroke(CaperTheme.green, lineWidth: 2) } }
            .background { if speaking { Circle().fill(CaperTheme.green.opacity(0.2)).padding(-3) } }
    }
}

private final class CaperResourceAnchor: NSObject {}
private var caperResourceBundle: Bundle {
    #if SWIFT_PACKAGE
    return .module
    #else
    return Bundle(for: CaperResourceAnchor.self)
    #endif
}

extension CaperAvatar {
    /// Resolve the framework/package asset explicitly. A missing catalog must
    /// show initials rather than a blank frame that still takes up avatar space.
    #if os(iOS)
    @MainActor static func image(for avatarID: Int?) -> UIImage? {
        guard let index = index(for: avatarID) else { return nil }
        return UIImage(named: "caper-avatar-\(index)", in: caperResourceBundle, compatibleWith: nil)
    }
    #else
    @MainActor static func image(for avatarID: Int?) -> NSImage? {
        guard let index = index(for: avatarID) else { return nil }
        return caperResourceBundle.image(forResource: NSImage.Name("caper-avatar-\(index)"))
    }
    #endif
}

private struct ConversationStage: View {
    @Bindable var model: AppModel
    let narrow: Bool
    let browse: () -> Void
    var createChannel: () -> Void = {}
    let membersVisible: Bool
    let toggleMembers: () -> Void
    var body: some View {
        if model.selectedChannelID == nil {
            VStack(spacing: 8) {
                Button(action: browse) { Label("Browse spaces", systemImage: "number") }.buttonStyle(CaperSecondaryButton())
                CaperIcon(name: "hash", size: 30).foregroundStyle(CaperTheme.terracottaBright)
                Text("No joined channels").font(CaperTheme.font(20, weight: .bold))
                Text("Browse public channels or accept a private invitation from the channel list.")
                    .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                if model.isOwner { Button("Create channel", action: createChannel).buttonStyle(VoiceJoinButton()).padding(.top, 6) }
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
    /// Web shows Connecting…/Offline only after a second without the gateway.
    @State private var showConnectionStatus = false
    @State private var joining = false
    @State private var joinError: String?
    @State private var confirmLeave = false

    /// Scroll the unsent message into view after the current layout pass, so a
    /// row that just grew (its error and actions appeared) is fully visible.
    private func revealPending(_ proxy: ScrollViewProxy) {
        guard let id = chat.pendingMessage?.id else { return }
        Task { @MainActor in
            await Task.yield()
            proxy.scrollTo("pending-\(id)", anchor: .bottom)
        }
    }
    init(model: AppModel, narrow: Bool, browse: @escaping () -> Void, membersVisible: Bool, toggleMembers: @escaping () -> Void) {
        self.model = model; chat = model.chat; voice = model.voice; self.narrow = narrow; self.browse = browse
        self.membersVisible = membersVisible; self.toggleMembers = toggleMembers
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: narrow ? 5 : 10) {
                if narrow {
                    // Web's narrow toggle: the menu icon with a visible "Browse" label.
                    Button(action: browse) {
                        HStack(spacing: 6) {
                            CaperIcon(name: "menu", size: 15)
                            Text("Browse").font(CaperTheme.font(11, weight: .bold))
                        }.foregroundStyle(CaperTheme.muted).padding(.horizontal, 9).frame(height: 34)
                            .overlay(RoundedRectangle(cornerRadius: 6).stroke(CaperTheme.border, lineWidth: 1))
                            .frame(minHeight: 44).contentShape(Rectangle())
                    }.buttonStyle(.plain).accessibilityLabel("Browse")
                }
                Text("# \(chat.channelName.lowercased())").font(CaperTheme.font(14, weight: .medium)).lineLimit(1)
                    .accessibilityLabel("# \(chat.channelName.lowercased())")
                    .accessibilityIdentifier("selected-channel-name")
                Spacer()
                if let channel = model.selectedChannel, channel.joined, model.detail?.space.demo != true {
                    Button("Leave channel") { confirmLeave = true }.font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                }
                if chat.liveState != .connected && showConnectionStatus {
                    Text(chat.liveState == .disconnected ? "Offline" : "Connecting…").font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.muted)
                        .accessibilityIdentifier("chat-connection-status")
                }
                Button(action: toggleMembers) { CaperIcon(name: "users", size: 20) }
                    .buttonStyle(SidebarIconButton()).help(membersVisible ? "Hide member list" : "Show member list")
                    .accessibilityLabel(membersVisible ? "Hide member list" : "Show member list")
            }.padding(.leading, narrow ? 13 : 18).padding(.trailing, 18).frame(height: 50)
                .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }

            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 0) {
                        if !chat.loadFailed { HStack(spacing: 6) {
                            if chat.olderError != nil {
                                Text("Couldn’t load older messages.")
                                Button("Retry") { Task { await chat.loadOlder() } }.disabled(chat.loadingOlder)
                                    .accessibilityIdentifier("load-older-messages")
                            } else if chat.hasMore {
                                Button(chat.loadingOlder ? "Loading…" : "Load older messages") {
                                    Task { await chat.loadOlder() }
                                }.disabled(chat.loadingOlder)
                                    .accessibilityIdentifier("load-older-messages")
                            }
                            else { Text("Beginning of conversation") }
                        }.font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.muted).frame(height: 44) }
                        if chat.loading && chat.messages.isEmpty {
                            Text("Loading messages…").font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).padding(.top, 80)
                        }
                        ForEach(Array(chat.messages.enumerated()), id: \.element.id) { index, message in
                            if index == 0 || !ChatDateDivider.sameLocalDay(chat.messages[index - 1].createdAt, message.createdAt) {
                                ChatDateDivider(createdAt: message.createdAt)
                            }
                            MessageRow(message: message, chat: chat).id(message.id)
                        }
                        if let pending = chat.pendingMessage {
                            if chat.messages.last.map({ ChatDateDivider.sameLocalDay($0.createdAt, pending.createdAt) }) != true {
                                ChatDateDivider(createdAt: pending.createdAt)
                            }
                            PendingMessageRow(pending: pending, author: chat.currentAuthor, error: chat.error,
                                              rejected: chat.sendRejected, canEdit: chat.draft.isEmpty,
                                              retry: { Task { await chat.send() } },
                                              edit: { _ = chat.discardRejected(edit: true) },
                                              dismiss: { _ = chat.discardRejected() })
                                .id("pending-\(pending.id)")
                        }
                        if chat.loadFailed, let error = chat.error {
                            // Web's failed first load: the error with Try again, in place of the conversation.
                            VStack(spacing: 10) {
                                Text(error).font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).multilineTextAlignment(.center)
                                Button("Try again") { Task { await chat.retryLoad() } }.buttonStyle(VoiceJoinButton())
                                    .accessibilityIdentifier("chat-try-again")
                            }.padding(.top, 80).padding(.horizontal, 24)
                        } else if chat.messages.isEmpty && !chat.loading && chat.pendingMessage == nil {
                            VStack(spacing: 7) {
                                Text("No messages yet.").font(CaperTheme.font(14, weight: .medium))
                                Text("Start the conversation in #\(chat.channelName.lowercased()).").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                            }.padding(.top, 80)
                        }
                    }
                }
                .accessibilityIdentifier("chat-timeline")
                #if os(macOS)
                // Web: End in the message list jumps to the latest message.
                .focusable().focusEffectDisabled()
                .onKeyPress(.end) {
                    guard let id = chat.messages.last?.id else { return .ignored }
                    proxy.scrollTo(id, anchor: .bottom)
                    return .handled
                }
                #endif
                .onChange(of: chat.messages.last?.id) { _, id in if let id { proxy.scrollTo(id, anchor: .bottom) } }
                .onChange(of: chat.pendingMessage?.id, initial: true) { _, _ in revealPending(proxy) }
                // A rejection adds the "Not sent" line with Edit and Dismiss after
                // the first scroll, so the row grows below the viewport; reveal it again.
                .onChange(of: chat.sendRejected) { _, _ in revealPending(proxy) }
                .onChange(of: chat.error) { _, _ in revealPending(proxy) }
            }

            HStack(spacing: 7) {
                if !chat.typingNames.isEmpty {
                    TypingDots()
                    Text(typingLabel).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                }
                Spacer()
            }.padding(.horizontal, 18).frame(height: 20)

            if let sessionError = chat.sessionError {
                HStack(spacing: 8) {
                    Text(sessionError).font(CaperTheme.font(11)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51))
                    Button("Retry session") { Task { await chat.retrySession() } }.buttonStyle(.plain)
                        .font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.text)
                    Spacer()
                }.padding(.horizontal, 18)
            }
            if let error = chat.error, chat.pendingMessage == nil, !chat.loadFailed {
                Text(error).font(CaperTheme.font(11)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)).frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18)
            }
            Group {
            if model.previewingChannel {
                VStack(alignment: .leading, spacing: 8) {
                HStack {
                    VStack(alignment: .leading, spacing: 5) {
                        Text("Preview").font(CaperTheme.font(12, weight: .bold))
                        (Text("Join ") + Text("#\(chat.channelName)").font(CaperTheme.font(12, weight: .bold)) + Text(" to interact with people here"))
                            .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    }
                    Spacer()
                    Button(joining ? "Joining…" : "Join channel") {
                        guard !joining, let channel = model.selectedChannel else { return }
                        joining = true; joinError = nil
                        Task { do { try await model.joinChannel(channel) } catch { joinError = error.localizedDescription }; joining = false }
                    }.buttonStyle(CaperPrimaryButton()).disabled(joining)
                }
                if let joinError { Text(joinError).font(CaperTheme.font(11)).foregroundStyle(.red) }
                }.padding(12)
            } else { HStack(alignment: .bottom, spacing: 8) {
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
                        // Size the actual label, not only its styled background: clicks
                        // beside the glyph must submit too, especially on macOS.
                        .frame(width: 42, height: 42)
                        .contentShape(Rectangle())
                }
                .buttonStyle(PrimaryIconButton())
                .disabled(chat.sending || chat.sendRejected || (chat.pendingMessage == nil && MessageValidation.error(for: chat.draft) != nil))
                .help(chat.sending ? "Sending…" : chat.sendRejected ? "Edit or dismiss the rejected message before sending another." : "Send message")
                .accessibilityLabel("Send message")
                .accessibilityValue(chat.sending ? "Sending" : "")
                .accessibilityIdentifier("send-message-button")
            }
            }
            }.padding(.horizontal, 18).padding(.vertical, 12)
            .sheet(isPresented: $confirmLeave) {
                if let channel = model.selectedChannel {
                    ConfirmationSheet(title: "Leave #\(channel.name)?", detail: channel.private && !model.isOwner ? "You’ll lose access and need another invitation to return. You’ll disconnect from this channel’s voice call." : "It will leave your sidebar. You can preview and rejoin from Browse channels. You’ll disconnect from this channel’s voice call.", action: "Leave channel", close: { confirmLeave = false }) { try await model.leaveChannel(channel) }
                }
            }
            if chat.draft.unicodeScalars.count >= 3000 {
                Text("\(chat.draft.unicodeScalars.count.formatted()) / 4,000").font(CaperTheme.font(10)).foregroundStyle(counterTone).padding(.bottom, 6)
            }
        }.background(CaperTheme.conversation)
            .task(id: chat.liveState) {
                showConnectionStatus = false
                guard chat.liveState != .connected else { return }
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                showConnectionStatus = true
            }
    }
    /// Web's counter tones at 3500 / 3750 / 3900 characters.
    private var counterTone: Color {
        let count = chat.draft.unicodeScalars.count
        if count >= 3900 { return Color(red: 1, green: 130/255, blue: 124/255) }
        if count >= 3750 { return Color(red: 237/255, green: 163/255, blue: 97/255) }
        if count >= 3500 { return Color(red: 228/255, green: 199/255, blue: 106/255) }
        return CaperTheme.muted
    }
    private var typingLabel: String {
        if chat.typingNames.count > 2 { return "Several people are typing…" }
        let names = chat.typingNames.joined(separator: " and ")
        return "\(names) \(chat.typingNames.count == 1 ? "is" : "are") typing…"
    }
}

/// Web's three bouncing typing dots (chat.css typing-bounce).
private struct TypingDots: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        TimelineView(.animation(paused: reduceMotion)) { timeline in
            HStack(spacing: 3) {
                ForEach(0..<3, id: \.self) { index in
                    let phase = reduceMotion ? 0 : Self.bounce(timeline.date.timeIntervalSinceReferenceDate - Double(index) * 0.15)
                    Circle().frame(width: 4, height: 4).opacity(0.45 + 0.55 * phase).offset(y: -3 * phase)
                }
            }.foregroundStyle(CaperTheme.muted).accessibilityHidden(true)
        }
    }
    /// 0→1→0 over the first 60% of a 1.2 s cycle, like the web keyframes.
    static func bounce(_ time: Double) -> Double {
        let t = (time.truncatingRemainder(dividingBy: 1.2) + 1.2).truncatingRemainder(dividingBy: 1.2) / 1.2
        guard t < 0.6 else { return 0 }
        return t < 0.3 ? t / 0.3 : (0.6 - t) / 0.3
    }
}

/// Web's Join tooltip while voice availability is unknown or off.
@MainActor private func voiceAvailabilityHelp(_ model: AppModel) -> String? {
    switch model.voiceAvailable {
    case true?: return nil
    case false?: return "Joining is not available at this time."
    case nil: return "Checking voice availability…"
    }
}

private struct PrimaryIconButton: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.foregroundStyle(.white)
            .background(configuration.isPressed ? CaperTheme.terracottaBright : CaperTheme.terracotta)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .opacity(isEnabled ? 1 : 0.45)
            .modifier(ControlHover())
    }
}

struct ChatDateDivider: View {
    let createdAt: String

    var body: some View {
        if let label = Self.fullDateLabel(createdAt) {
            HStack(spacing: 10) {
                Rectangle().fill(CaperTheme.border).frame(height: 1)
                Text(label).font(CaperTheme.font(10, weight: .bold)).foregroundStyle(CaperTheme.muted)
                    .fixedSize(horizontal: true, vertical: false)
                Rectangle().fill(CaperTheme.border).frame(height: 1)
            }.padding(.horizontal, 18).padding(.vertical, 8)
        }
    }

    static func date(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fractional.date(from: value) ?? ISO8601DateFormatter().date(from: value)
    }

    static func sameLocalDay(_ first: String, _ second: String, calendar: Calendar = .current) -> Bool {
        guard let firstDate = date(first), let secondDate = date(second) else { return false }
        return calendar.isDate(firstDate, inSameDayAs: secondDate)
    }

    static func fullDateLabel(_ value: String, locale: Locale = .current, timeZone: TimeZone = .current) -> String? {
        guard let date = date(value) else { return nil }
        let formatter = DateFormatter()
        formatter.locale = locale
        formatter.timeZone = timeZone
        formatter.dateStyle = .full
        formatter.timeStyle = .none
        return formatter.string(from: date)
    }
}

private struct MessageRow: View {
    let message: ChatMessage
    @Bindable var chat: ChatModel
    @State private var pickerVisible = false
    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Avatar(name: message.author.name, size: 34, avatarID: message.author.avatarId)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 7) {
                    Text(message.author.name).font(CaperTheme.font(13, weight: .bold))
                    if message.author.isGuest { Text("GUEST").font(CaperTheme.font(9, weight: .bold)).foregroundStyle(CaperTheme.muted).padding(.horizontal, 5).overlay(RoundedRectangle(cornerRadius: 4).stroke(CaperTheme.border)) }
                    Text(timeLabel(message.createdAt)).font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
                }
                Text(message.content.text).font(CaperTheme.font(14)).foregroundStyle(Color(red: 222/255, green: 223/255, blue: 224/255)).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                ReactionRow(message: message, chat: chat) { pickerVisible = true }
                if let error = chat.reactionErrors[message.id] {
                    HStack(spacing: 8) {
                        Text(error)
                        Button("Retry") { Task { await chat.retryReaction(messageID: message.id) } }
                            .disabled(chat.isPreview || chat.currentAuthor == nil)
                    }.font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.terracottaBright)
                }
            }
        }.padding(.horizontal, 18).padding(.vertical, 10)
            .onChange(of: chat.isPreview) { _, preview in
                if preview { pickerVisible = false }
            }
            .sheet(isPresented: $pickerVisible) {
                ReactionPicker { emoji in
                    pickerVisible = false
                    Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: true) }
                }
            }
    }
    private func timeLabel(_ value: String) -> String {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let date = fractional.date(from: value) ?? ISO8601DateFormatter().date(from: value) else { return "" }
        return date.formatted(date: .omitted, time: .shortened)
    }
}

private struct ReactionRow: View {
    let message: ChatMessage
    @Bindable var chat: ChatModel
    let showPicker: () -> Void

    var body: some View {
        ReactionFlowLayout(spacing: 6) {
            ForEach(message.reactions ?? []) { reaction in
                ReactionChip(messageID: message.id, reaction: reaction, chat: chat)
            }
            Button(action: showPicker) {
                Image(systemName: "face.smiling").font(.system(size: 16, weight: .medium))
                    .frame(width: 28, height: 28)
                    #if os(iOS)
                    .frame(minWidth: 44, minHeight: 44)
                    #endif
            }
            .buttonStyle(.plain)
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
            .disabled(chat.isPreview || chat.reactionSaving.contains(message.id) || chat.currentAuthor == nil)
            .accessibilityLabel("Add reaction")
            .accessibilityIdentifier("add-reaction-\(message.id)")
            if chat.reactionSaving.contains(message.id) {
                ProgressView().controlSize(.small).accessibilityLabel("Saving reaction")
            }
        }
        .accessibilityIdentifier("reaction-row-\(message.id)")
    }
}

private struct ReactionChip: View {
    let messageID: String
    let reaction: MessageReaction
    @Bindable var chat: ChatModel
    private var own: Bool { chat.currentAuthor.map { reaction.authorIds.contains($0.id) } ?? false }

    var body: some View {
        Button {
            Task { await chat.setReaction(messageID: messageID, emoji: reaction.emoji, active: !own) }
        } label: {
            HStack(spacing: 4) {
                EmojiArtworkView(emoji: reaction.emoji, size: 18)
                Text("\(reaction.authorIds.count)").font(CaperTheme.font(11, weight: .bold))
            }
            .padding(.horizontal, 7).frame(height: 28)
            #if os(iOS)
            .frame(minHeight: 44)
            #endif
            .background(own ? CaperTheme.terracotta.opacity(0.24) : CaperTheme.surface)
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(own ? CaperTheme.terracottaBright : CaperTheme.border))
        }
        .buttonStyle(.plain)
        .disabled(chat.isPreview || chat.reactionSaving.contains(messageID) || chat.currentAuthor == nil)
        .accessibilityLabel("\(reaction.emoji) reaction, \(reaction.authorIds.count), \(own ? "selected by you" : "not selected by you")")
        .accessibilityAddTraits(own ? .isSelected : [])
    }
}

private struct ReactionFlowLayout: Layout {
    let spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        layout(subviews: subviews, width: proposal.width ?? .infinity).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let result = layout(subviews: subviews, width: bounds.width)
        for (index, point) in result.points.enumerated() {
            subviews[index].place(at: CGPoint(x: bounds.minX + point.x, y: bounds.minY + point.y), anchor: .topLeading, proposal: .unspecified)
        }
    }

    private func layout(subviews: Subviews, width: CGFloat) -> (size: CGSize, points: [CGPoint]) {
        var x: CGFloat = 0, y: CGFloat = 0, rowHeight: CGFloat = 0
        var points: [CGPoint] = []
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width { x = 0; y += rowHeight + spacing; rowHeight = 0 }
            points.append(CGPoint(x: x, y: y))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
        }
        return (CGSize(width: width.isFinite ? width : max(0, x - spacing), height: y + rowHeight), points)
    }
}

private struct ReactionPicker: View {
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    let select: (String) -> Void
    private var choices: [EmojiCatalogEntry] {
        let term = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !term.isEmpty else { return EmojiArtwork.choices }
        return EmojiArtwork.choices.filter { $0.name.lowercased().contains(term) || $0.keywords.lowercased().contains(term) }
    }
    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                #if os(iOS)
                TextField("Search emoji", text: $query)
                    .textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier("reaction-picker-search")
                    .padding(12)
                #endif
                if choices.isEmpty {
                    ContentUnavailableView("No emoji found", systemImage: "magnifyingglass", description: Text("Try another search."))
                        .accessibilityIdentifier("reaction-picker-empty")
                } else {
                    ScrollView {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 42), spacing: 8)], spacing: 8) {
                            ForEach(choices) { entry in
                                Button { select(entry.emoji) } label: {
                                    EmojiArtworkView(emoji: entry.emoji, size: 30).frame(width: 42, height: 42)
                                }.buttonStyle(.plain).accessibilityLabel(entry.name)
                            }
                        }.padding(12)
                    }.accessibilityIdentifier("reaction-picker-grid")
                }
            }
            .navigationTitle("Add reaction")
            #if os(macOS)
            .searchable(text: $query, prompt: "Search emoji")
            #endif
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        }.frame(minWidth: 320, minHeight: 420)
    }
}

private struct PendingMessageRow: View {
    let pending: PendingMessage; let author: ChatAuthor?; let error: String?
    let rejected: Bool; let canEdit: Bool; let retry: () -> Void; let edit: () -> Void; let dismiss: () -> Void
    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Avatar(name: author?.name ?? "Guest", size: 34, avatarID: author?.avatarId)
            VStack(alignment: .leading, spacing: 4) {
                Text(author?.name ?? "Guest").font(CaperTheme.font(13, weight: .bold))
                Text(pending.text).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted)
                if let error {
                    VStack(alignment: .leading, spacing: 5) {
                        Text("\(rejected ? "Not sent." : "Not confirmed yet.") \(error)")
                        HStack(spacing: 12) {
                            if rejected {
                                Button("Edit", action: edit).disabled(!canEdit)
                                    .help(canEdit ? "" : "Clear your current draft to edit this message.")
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
    @FocusState private var displayNameFocused: Bool
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                Wordmark()
                Text("ONE LAST THING").font(CaperTheme.font(12, weight: .bold)).tracking(2).foregroundStyle(CaperTheme.muted)
                    .padding(.top, 20)
                Text("Choose how you show up.").font(CaperTheme.font(28, weight: .bold))
                    .fixedSize(horizontal: false, vertical: true)
                Text("Your username is unique. Your display name is what people see in conversations.")
                    .font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted).fixedSize(horizontal: false, vertical: true)
                CaperField(title: "Username", text: $username)
                    .submitLabel(.next).onSubmit { displayNameFocused = true }
                Text("3-32 lowercase letters, numbers, or underscores.")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                CaperField(title: "Display name", text: $displayName, focus: $displayNameFocused)
                Text("Shown to other people. It does not need to be unique.")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                if let error = model.error {
                    Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.terracottaBright)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Button(model.busy ? "Saving…" : "Finish account") { Task { await model.saveProfile(username: username, displayName: displayName) } }
                    .buttonStyle(CaperPrimaryButton())
                    .disabled(model.busy || ProfileValidation.error(username: username, displayName: displayName) != nil)
                    .accessibilityIdentifier("profile-continue")
                HStack {
                    Spacer()
                    Button("Log out") { Task { await model.logout() } }.buttonStyle(.plain)
                        .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                }
            }.padding(28).frame(maxWidth: 440).frame(maxWidth: .infinity)
        }.background(CaperTheme.blackout)
    }
}

private struct WorkspaceSheetView: View {
    let item: WorkspaceSheet
    @Bindable var model: AppModel
    let close: () -> Void
    var body: some View {
        Group {
            switch item {
            case .login: EmptyView() // Presented full-screen by LoginPresentation.
            case .profile: ProfileSheet(model: model, close: close)
            case .createSpace: SpaceEditor(model: model, close: close, managing: false)
            case .createChannel: ChannelEditor(model: model, channel: nil, close: close)
            case .manageSpace: SpaceEditor(model: model, close: close, managing: true)
            case .manageChannel(let channel): ChannelEditor(model: model, channel: channel, close: close)
            case .invitation(let space): InvitationSheet(model: model, invitation: space, close: close)
            case .channelInvitation(let invitation): ChannelInvitationSheet(model: model, invitation: invitation, close: close)
            case .leaveSpace: ConfirmationSheet(title: "Leave \(model.detail?.space.name ?? "space")?", detail: "You will lose access to its channels and conversations. An owner can add you again later.", action: "Leave space", close: close) { try await model.leaveCurrentSpace() }
            case .audio: AudioPreferencesView(voice: model.voice, debugEnabled: model.account?.debugEnabled == true, close: close)
            case .connection: ScrollView { ConnectionDetailsView(voice: model.voice, close: close) }
            case .diagnostics:
                VStack(alignment: .leading, spacing: 18) {
                    SheetHeader(title: "Audio diagnostics", detail: "Local processing counters", close: close)
                    ScrollView {
                        if model.account?.debugEnabled == true { AudioDiagnosticsView(voice: model.voice).padding(22) }
                    }
                }
            }
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }
}

private struct ChannelInvitationSheet: View {
    @Bindable var model: AppModel
    let invitation: ChannelInvitation
    let close: () -> Void
    @State private var pending = false
    @State private var error: String?
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: "Private channel invitation", detail: "#\(invitation.channel.name) in \(model.detail?.space.name ?? "this space")", close: { if !pending { close() } })
            VStack(alignment: .leading, spacing: 14) {
                Text("Invited by \(invitation.inviter.displayName) (@\(invitation.inviter.username)). This invitation expires after 7 days.")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                Text("Messages stay hidden until you accept. Acceptance joins the channel; it does not enter voice.").font(CaperTheme.font(12))
                if let error { Text(error).foregroundStyle(.red) }
                HStack { Spacer(); Button("Decline") { run { try await model.declineChannelInvitation(invitation) } }.buttonStyle(CaperSecondaryButton()); Button("Accept") { run { try await model.acceptChannelInvitation(invitation) } }.buttonStyle(CaperPrimaryButton()) }.disabled(pending)
            }.padding(22)
        }.background(CaperTheme.surface).interactiveDismissDisabled(pending)
    }
    private func run(_ action: @escaping () async throws -> Void) {
        guard !pending else { return }; pending = true; error = nil
        Task { do { try await action(); close() } catch { self.error = error.localizedDescription }; pending = false }
    }
}

private struct SheetHeader: View {
    let title: String; var detail: String?; var closeLabel = "Close"; var titleIcon: Image? = nil; let close: () -> Void
    var body: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 12) {
                    if let titleIcon { titleIcon.resizable().interpolation(.high).frame(width: 32, height: 32).accessibilityHidden(true) }
                    Text(title).font(CaperTheme.font(20, weight: .bold))
                }
                if let detail { Text(detail).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted) }
            }
            Spacer(); Button(action: close) { CaperIcon(name: "x") }.buttonStyle(SidebarIconButton()).accessibilityLabel(closeLabel)
        }.padding(22).overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
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
                Text(model.challengeID == nil ? "Welcome to Caper" : "Check your email.")
                    .font(CaperTheme.font(52, weight: .black)).tracking(-2.5).padding(.bottom, 8)
                Text(model.challengeID == nil ? "Use your email to create an account or return to one. We’ll send a code to your email." : "Enter the six-character code sent to \(email.trimmingCharacters(in: .whitespacesAndNewlines)). It expires in 10 minutes.")
                    .font(CaperTheme.font(16)).foregroundStyle(CaperTheme.muted).lineSpacing(7).padding(.bottom, 30)
                if model.challengeID == nil {
                    CaperField(title: "Email address", text: $email, placeholder: "you@example.com")
                    if let error = model.error { LoginError(message: error).padding(.top, 18) }
                    Button { Task { await model.requestCode(email: email) } } label: {
                        HStack { Text(model.busy ? "Sending…" : "Email me a code"); Spacer(); Image(systemName: "arrow.right") }
                    }.buttonStyle(LoginActionButton()).disabled(model.busy || email.isEmpty).padding(.top, model.error == nil ? 28 : 28)
                } else {
                    CaperField(title: "Sign-in code", text: $code)
                        .disabled(model.loginAttemptsRemaining == 0)
                        .onChange(of: code) { _, value in
                            // Web accepts the unambiguous code alphabet only, uppercased, six characters.
                            let allowed = Set("ABCDEFGHJKMNPQRSTWXYZ23456789")
                            let filtered = String(value.uppercased().filter { allowed.contains($0) }.prefix(6))
                            guard filtered != value else { return }
                            #if os(macOS)
                            // AppKit's field editor ignores a rewrite inside its own edit; apply it next turn.
                            DispatchQueue.main.async { code = filtered }
                            #else
                            code = filtered
                            #endif
                        }
                    if let error = model.error { LoginError(message: error).padding(.top, 18) }
                    if model.loginAttemptsRemaining == 1 {
                        Text("One attempt left. Check the code carefully.").font(CaperTheme.font(14, weight: .bold)).padding(.top, 12)
                    }
                    if model.loginAttemptsRemaining == 0 {
                        Button { code = ""; Task { await model.requestCode(email: email) } } label: {
                            HStack { Text(model.busy ? "Sending…" : "Email me a new code"); Spacer(); Image(systemName: "arrow.right") }
                        }.buttonStyle(LoginActionButton()).disabled(model.busy).padding(.top, 28)
                    } else {
                        Button { Task { await model.verify(code: code); if model.account != nil && model.phase != .onboarding { close() } } } label: {
                            HStack { Text(model.busy ? "Checking…" : "Continue"); Spacer(); Image(systemName: "arrow.right") }
                        }.buttonStyle(LoginActionButton()).disabled(model.busy || code.count != 6).padding(.top, 28)
                    }
                    Button("Use a different email") { model.challengeID = nil; model.error = nil }.buttonStyle(.plain).foregroundStyle(CaperTheme.muted).padding(.top, 18)
                }
            }
            .frame(maxWidth: 440)
            .padding(.horizontal, 20)
            #if os(iOS)
            .padding(.vertical, 32)
            #else
            .padding(.top, 108)
            #endif
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
    @FocusState private var displayNameFocused: Bool
    var body: some View {
        VStack(spacing: 0) {
            // Web's Edit profile dialog; Log out lives in User Settings.
            SheetHeader(title: "Edit profile", detail: "Your username is unique. Your display name is what people see in conversations.",
                        closeLabel: "Close profile", close: close)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    CaperField(title: "Username", text: $username)
                        .submitLabel(.next).onSubmit { displayNameFocused = true }
                    Text("3-32 lowercase letters, numbers, or underscores.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    CaperField(title: "Display name", text: $displayName, focus: $displayNameFocused)
                        // Return saves: on iPhone an error line can push Save
                        // profile under the keyboard.
                        .submitLabel(.done).onSubmit(save)
                    Text("Shown to other people. It does not need to be unique.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    Text(model.error ?? " ").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.terracottaBright)
                        .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
                        .opacity(model.error == nil ? 0 : 1).accessibilityHidden(model.error == nil)
                    Button(model.busy ? "Saving…" : "Save profile", action: save)
                        .buttonStyle(CaperPrimaryButton())
                        .disabled(!canSave)
                        .accessibilityIdentifier("profile-save")
                }.padding(22).frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxHeight: 500).scrollDismissesKeyboard(.interactively)
        }.background(CaperTheme.surface)
            .onAppear {
                username = model.account?.username ?? ""
                displayName = model.account?.displayName ?? ""
            }
    }

    private var canSave: Bool {
        !model.busy && ProfileValidation.error(username: username, displayName: displayName) == nil
    }

    private func save() {
        guard canSave else { return }
        Task {
            await model.saveProfile(username: username, displayName: displayName)
            if model.error == nil { close() }
        }
    }
}

private struct InvitationSheet: View {
    @Bindable var model: AppModel
    let invitation: Space
    let close: () -> Void
    @State private var pending = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            SheetHeader(title: "You’re invited!", detail: nil, titleIcon: Image("IncomingEnvelope", bundle: artworkBundle), close: close)
            Text("Join \(invitation.name)?").font(CaperTheme.font(20, weight: .bold)).padding(.horizontal, 22)
            if let inviter = invitation.inviter {
                Text("\(inviter.displayName) (@\(inviter.username)) invited you.").font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).padding(.horizontal, 22)
            }
            if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(.red).padding(.horizontal, 22) }
            HStack {
                Spacer()
                Button("Decline") { perform { try await model.declineInvitation(invitation); close() } }.buttonStyle(CaperSecondaryButton())
                Button("Accept") { perform { try await model.acceptInvitation(invitation); close() } }.buttonStyle(CaperPrimaryButton())
            }.disabled(pending).padding(22)
        }.background(CaperTheme.surface)
    }

    private var artworkBundle: Bundle {
        #if SWIFT_PACKAGE
        Bundle.module
        #else
        Bundle(for: CaperEffects.self)
        #endif
    }

    private func perform(_ action: @escaping () async throws -> Void) {
        pending = true; error = nil
        Task { do { try await action() } catch { self.error = error.localizedDescription }; pending = false }
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
                    Button(pending ? "Saving…" : managing ? "Save name" : "Create space") { run { if managing { try await model.renameSpace(name) } else { try await model.createSpace(name: name); close() } } }.buttonStyle(CaperPrimaryButton()).disabled(pending)
                    if managing {
                        Divider().overlay(CaperTheme.border)
                        Text("Members  \(model.detail?.members.count ?? 0)").font(CaperTheme.font(14, weight: .bold))
                            .accessibilityLabel("Members \(model.detail?.members.count ?? 0)")
                            .accessibilityIdentifier("space-members-heading")
                        HStack {
                            TextField("Exact username", text: Binding(get: { username }, set: { username = WorkspaceValidation.normalizeUsername($0) })).textFieldStyle(CaperTextFieldStyle())
                                .autocorrectionDisabled()
                                #if os(iOS)
                                .textInputAutocapitalization(.never)
                                #endif
                                .submitLabel(.done).onSubmit(addMember)
                                .accessibilityLabel("Exact username")
                            Button("Invite", action: addMember).buttonStyle(CaperSecondaryButton())
                                .disabled(WorkspaceValidation.usernameError(username) != nil)
                        }.disabled(pending)
                        ForEach(model.detail?.members ?? []) { member in
                            HStack { Avatar(name: member.displayName, size: 30, avatarID: member.avatarId); VStack(alignment: .leading) { Text(member.displayName); Text("@\(member.username)\(member.owner ? " · Owner" : "")").foregroundStyle(CaperTheme.muted) }; Spacer(); if !member.owner { Button("Remove") { run { try await model.removeSpaceMember(member) } } } }.font(CaperTheme.font(12))
                        }
                        Text("Pending invitations  \(model.pendingMembers.count)").font(CaperTheme.font(14, weight: .bold))
                        ForEach(model.pendingMembers) { member in
                            HStack { Avatar(name: member.displayName, size: 30); VStack(alignment: .leading) { Text(member.displayName); Text("@\(member.username)").foregroundStyle(CaperTheme.muted) }; Spacer(); Button("Cancel") { run { try await model.cancelSpaceInvitation(member) } } }.font(CaperTheme.font(12))
                        }
                        Divider().overlay(CaperTheme.border)
                        Text("Delete space").font(CaperTheme.font(14, weight: .bold))
                        Text("Delete this space and all its channels for every member.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                        Button("Delete space", role: .destructive) { CaperEffects.shared.play(.warning); confirmDelete = true }.buttonStyle(CaperSecondaryButton()).disabled(pending)
                    }
                    if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)) }
                }.padding(22)
            }.scrollDismissesKeyboard(.interactively).accessibilityIdentifier("space-settings-scroll")
        }.background(CaperTheme.surface).onAppear { name = managing ? model.detail?.space.name ?? "" : "" }
        .task(id: model.detail?.space.id) { if managing { do { try await model.loadSpaceInvitations() } catch { self.error = error.localizedDescription } } }
        .sheet(isPresented: $confirmDelete) {
            ConfirmationSheet(title: "Delete space", detail: "Delete \(model.detail?.space.name ?? name) for everyone? All its channels and their messages will disappear from the space. This cannot be undone.", action: "Delete space", close: { confirmDelete = false }) {
                try await model.deleteCurrentSpace()
                CaperEffects.shared.play(.delete)
                close()
            }
            .modifier(ConfirmationPresentation())
        }
    }
    private func addMember() {
        guard !pending, WorkspaceValidation.usernameError(username) == nil else { return }
        let submitted = username
        run { try await model.addSpaceMember(username: submitted); username = "" }
    }
    private func run(_ action: @escaping () async throws -> Void) { guard !pending else { return }; pending = true; error = nil; Task { do { try await action() } catch { self.error = error.localizedDescription }; pending = false } }
}

private struct ChannelEditor: View {
    @Bindable var model: AppModel; @State var channel: Channel?; let close: () -> Void
    @State private var name = ""; @State private var privateChannel = false; @State private var members: [Member] = []; @State private var username = ""; @State private var error: String?; @State private var pending = false
    @State private var confirmDelete = false
    @State private var membersError: String?
    @State private var invitations: [Member] = []
    @State private var memberError: String?
    @State private var loadingMembers = false
    @FocusState private var nameFocused: Bool
    private var dirty: Bool { channel.map { name != $0.name || privateChannel != $0.private } ?? false }
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: channel == nil ? "Create a channel" : "Overview", close: close)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    VStack(alignment: .leading, spacing: 7) {
                        Text("Channel name").font(CaperTheme.font(12, weight: .bold))
                        HStack(spacing: 8) {
                            if channel == nil { CaperIcon(name: privateChannel ? "lock" : "hash", size: 16).foregroundStyle(CaperTheme.muted) }
                            TextField("project-updates", text: Binding(get: { name }, set: { name = WorkspaceValidation.normalizeChannelName($0) }))
                                .focused($nameFocused).onSubmit(submit)
                        }.textFieldStyle(CaperTextFieldStyle())
                    }
                    if channel == nil {
                        Text("Channels are where conversations happen around a topic. Use a name that is easy to find and understand.")
                            .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).fixedSize(horizontal: false, vertical: true)
                    }
                    Toggle(isOn: Binding(get: { privateChannel }, set: { privateChannel = $0; CaperEffects.shared.toggle($0) })) { VStack(alignment: .leading) { Text("Private channel").font(CaperTheme.font(13, weight: .bold)); Text(privateChannel ? "Only you and the people you add can view or join." : "Anyone in \(model.detail?.space.name ?? "this space") can view or join this channel.").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted) }.frame(minHeight: channel == nil ? nil : 32, alignment: .topLeading) }.toggleStyle(.switch)
                    if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)) }
                    if channel == nil {
                        HStack {
                            Spacer()
                            Button("Cancel", action: close).buttonStyle(CaperSecondaryButton()).keyboardShortcut(.cancelAction)
                            Button(pending ? "Saving…" : "Create channel", action: submit).buttonStyle(CaperPrimaryButton()).frame(width: 160)
                                .disabled(pending).keyboardShortcut(.defaultAction)
                        }
                    }
                    if let channel, channel.private {
                        Divider().overlay(CaperTheme.border)
                        HStack { Text("Members").font(CaperTheme.font(14, weight: .bold)); Text("\(members.count)").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted) }
                        if loadingMembers { ProgressView("Loading members…") }
                        if let membersError {
                            Text(membersError).foregroundStyle(.red)
                            Button("Retry loading members") { Task { await loadMembers(channel) } }.disabled(loadingMembers)
                        }
                        HStack {
                            TextField("Exact username", text: Binding(get: { username }, set: { username = WorkspaceValidation.normalizeUsername($0) })).textFieldStyle(CaperTextFieldStyle())
                                .autocorrectionDisabled()
                                #if os(iOS)
                                .textInputAutocapitalization(.never)
                                #endif
                                .submitLabel(.done).onSubmit { addMember(channel) }
                                .accessibilityLabel("Exact username")
                            Button("Invite") { addMember(channel) }.buttonStyle(CaperSecondaryButton())
                                .disabled(WorkspaceValidation.usernameError(username) != nil)
                        }
                            .disabled(pending || loadingMembers || membersError != nil)
                        if let memberError { Text(memberError).font(CaperTheme.font(12)).foregroundStyle(Color(red: 1, green: 0.61, blue: 0.51)) }
                        ForEach(members) { member in
                            HStack {
                                Avatar(name: member.displayName, size: 30, avatarID: member.avatarId)
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(member.displayName).font(CaperTheme.font(12, weight: .bold))
                                    Text("@\(member.username)\(member.owner ? " · Owner" : "")").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                                }
                                Spacer()
                                if !member.owner { Button("Remove") { run { try await model.removeChannelMember(channel, member: member); members.removeAll { $0.id == member.id } } } }
                            }.font(CaperTheme.font(12))
                        }.disabled(pending || loadingMembers || membersError != nil)
                        Text("Pending invitations  \(invitations.count)").font(CaperTheme.font(14, weight: .bold))
                        ForEach(invitations) { member in
                            HStack { Text("\(member.displayName) (@\(member.username))"); Spacer(); Button("Cancel") { run { try await model.removeChannelMember(channel, member: member); invitations.removeAll { $0.id == member.id } } } }
                                .font(CaperTheme.font(12))
                        }.disabled(pending || loadingMembers || membersError != nil)
                    }
                    if channel != nil {
                        Divider().overlay(CaperTheme.border)
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Delete channel").font(CaperTheme.font(14, weight: .bold))
                            Text("Delete this channel for everyone in the space.").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                            Button("Delete channel", role: .destructive) { CaperEffects.shared.play(.warning); confirmDelete = true }.buttonStyle(CaperSecondaryButton()).disabled(pending)
                        }
                    }
                }.padding(22)
            }
            if channel != nil {
                // Reserve the save bar while clean so toggling privacy cannot resize the sheet.
                let saveBar = HStack {
                    Text("You have unsaved changes.").font(CaperTheme.font(12))
                    Spacer()
                    Button("Reset") { name = channel?.name ?? ""; privateChannel = channel?.private ?? false; error = nil }.buttonStyle(CaperSecondaryButton()).disabled(pending)
                    Button(pending ? "Saving…" : "Save changes", action: submit).buttonStyle(CaperPrimaryButton()).frame(width: 150).disabled(pending)
                }.padding(.horizontal, 22).padding(.vertical, 12).background(CaperTheme.raised)
                    .accessibilityIdentifier("channel-save-bar")
                if dirty { saveBar } else { saveBar.hidden().allowsHitTesting(false) }
            }
        }.background(CaperTheme.surface).onAppear { name = channel?.name ?? ""; privateChannel = channel?.private ?? false; if channel == nil { nameFocused = true } }
        .task(id: channel?.private) { if let channel, channel.private { await loadMembers(channel) } }
        .sheet(isPresented: $confirmDelete) {
            if let channel {
                ConfirmationSheet(title: "Delete channel", detail: "Delete #\(channel.name) for everyone? This channel and its messages will disappear from the space. This cannot be undone.", action: "Delete channel", close: { confirmDelete = false }) {
                    try await model.deleteChannel(channel)
                    CaperEffects.shared.play(.delete)
                    close()
                }
                .modifier(ConfirmationPresentation())
            }
        }
    }
    private func submit() {
        guard !pending else { return }
        if let existing = channel {
            guard dirty else { return }
            run { let updated = try await model.updateChannel(existing, name: name, privateChannel: privateChannel); channel = updated; name = updated.name; privateChannel = updated.private }
        } else {
            run {
                let created = try await model.createChannel(name: name, privateChannel: privateChannel)
                // Web opens a new private channel's overview so people can be added.
                if let created, created.private { channel = created; name = created.name; privateChannel = created.private } else { close() }
            }
        }
    }
    private func addMember(_ channel: Channel) {
        guard !pending, !loadingMembers, membersError == nil else { return }
        guard !username.isEmpty else { memberError = "Enter an exact username."; return }
        memberError = nil
        run { let member = try await model.addChannelMember(channel, username: username); invitations.removeAll { $0.id == member.id }; invitations.append(member); username = "" }
    }
    private func loadMembers(_ channel: Channel) async {
        guard !loadingMembers else { return }
        loadingMembers = true; membersError = nil
        defer { loadingMembers = false }
        do { members = try await model.channelMembers(channel); invitations = try await model.channelInvitations(channel) }
        catch { membersError = error.localizedDescription }
    }
    private func run(_ action: @escaping () async throws -> Void) { pending = true; error = nil; Task { do { try await action() } catch { self.error = error.localizedDescription }; pending = false } }
}

private struct DialogDismissDisabled: PreferenceKey {
    static let defaultValue = false
    static func reduce(value: inout Bool, nextValue: () -> Bool) { value = value || nextValue() }
}

private struct ConfirmationSheet: View {
    let title: String; let detail: String; let action: String; let close: () -> Void; let perform: () async throws -> Void
    @State private var pending = false; @State private var error: String?
    var body: some View { VStack(spacing: 0) { SheetHeader(title: title, detail: detail, close: { if !pending { close() } }); ScrollView { VStack(spacing: 16) { if let error { Text(error).foregroundStyle(.red) }; HStack { Button("Cancel", action: close).disabled(pending).keyboardShortcut(.cancelAction); Button(pending ? (action.hasPrefix("Delete") ? "Deleting…" : "Saving…") : action, role: .destructive) { guard !pending else { return }; pending = true; error = nil; Task { do { try await perform(); close() } catch { self.error = error.localizedDescription }; pending = false } }.disabled(pending).accessibilityIdentifier("confirm-destructive-action") } }.padding(22) } }.background(CaperTheme.surface).interactiveDismissDisabled(pending).preference(key: DialogDismissDisabled.self, value: pending) }
}

private struct ConfirmationPresentation: ViewModifier {
    func body(content: Content) -> some View {
        content
            .frame(maxWidth: 560, maxHeight: .infinity, alignment: .top)
            #if os(iOS)
            .presentationDetents([.medium])
            #else
            .frame(width: 520, height: 280, alignment: .top)
            #endif
    }
}

private struct CaperField: View {
    let title: String; @Binding var text: String; var placeholder: String?
    /// Lets a form move focus between fields, e.g. Return on Username focuses
    /// Display name so the keyboard never hides the next field on iPhone.
    var focus: FocusState<Bool>.Binding?
    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(title).font(CaperTheme.font(12, weight: .bold))
            let field = TextField(placeholder ?? title, text: $text).textFieldStyle(CaperTextFieldStyle()).accessibilityLabel(title)
            if let focus { field.focused(focus) } else { field }
        }
    }
}

private struct CaperTextFieldStyle: TextFieldStyle {
    func _body(configuration: TextField<Self._Label>) -> some View { configuration.textFieldStyle(.plain).font(CaperTheme.font(14)).padding(.horizontal, 11).frame(height: 42).background(CaperTheme.composer).overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border)).clipShape(RoundedRectangle(cornerRadius: 8)) }
}

private struct CaperPrimaryButton: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled
    func makeBody(configuration: Configuration) -> some View { configuration.label.font(CaperTheme.font(13, weight: .bold)).foregroundStyle(.white).frame(maxWidth: .infinity).frame(height: 42).background(configuration.isPressed ? CaperTheme.terracottaBright : CaperTheme.terracotta).clipShape(RoundedRectangle(cornerRadius: 8)).opacity(isEnabled ? 1 : 0.45).modifier(ControlHover()) }
}

private struct ControlHover: ViewModifier {
    @Environment(\.isEnabled) private var enabled
    @State private var hovered = false
    func body(content: Content) -> some View {
        content.overlay {
            RoundedRectangle(cornerRadius: 8).fill(Color.white.opacity(hovered && enabled ? 0.06 : 0))
                .allowsHitTesting(false)
        }
        .onHover { inside in
            hovered = inside
            #if os(macOS)
            if inside && enabled { NSCursor.pointingHand.set() } else { NSCursor.arrow.set() }
            #endif
        }
    }
}

private struct CaperSecondaryButton: ButtonStyle {
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(CaperTheme.font(12, weight: .medium))
            .foregroundStyle(configuration.role == .destructive ? CaperTheme.terracottaBright : CaperTheme.text)
            .padding(.horizontal, 12).padding(.vertical, 8)
            .background(configuration.isPressed ? CaperTheme.composer : CaperTheme.raised)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
            .opacity(enabled ? 1 : 0.45).modifier(ControlHover())
    }
}

/// Caper's slider: the platform's own control in Caper's colour. It keeps native
/// keyboard, VoiceOver and pointer behaviour. A drawn track and thumb only
/// imitated a slider to accessibility, which could not report its geometry, so
/// VoiceOver adjustment and UI automation could not move it.
private struct CaperSlider: View {
    @Binding var value: Double
    let bounds: ClosedRange<Double>
    let step: Double
    init(value: Binding<Double>, in bounds: ClosedRange<Double>, step: Double) {
        _value = value; self.bounds = bounds; self.step = step
    }
    var body: some View {
        Slider(value: $value, in: bounds, step: step)
            .tint(CaperTheme.terracottaBright)
            .frame(height: 32)
            .accessibilityValue("\(Int(value))%")
    }
}

private struct CaperDevicePicker: View {
    let title: String
    @Binding var selection: String
    let devices: [AudioDevice]
    var body: some View {
        Menu {
            Button("System default") { selection = "" }
            ForEach(devices) { device in Button(device.name) { selection = device.id } }
        } label: {
            HStack(spacing: 8) {
                Text(devices.first(where: { $0.id == selection })?.name ?? "System default").lineLimit(1)
                Spacer(minLength: 0)
                CaperIcon(name: "chevron-down", size: 12)
            }.font(CaperTheme.font(12)).foregroundStyle(CaperTheme.text)
                .padding(10).background(CaperTheme.composer)
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        }.menuStyle(.borderlessButton).menuIndicator(.hidden)
            .accessibilityLabel(title).accessibilityValue(devices.first(where: { $0.id == selection })?.name ?? "System default")
            .modifier(ControlHover())
    }
}

/// Web's "Audio test" (Call.tsx audio dialog + MicPlayback.tsx).
private struct AudioPreferencesView: View {
    @Bindable var voice: VoiceClient
    var debugEnabled = false
    let close: () -> Void
    @State private var controlsHeight: CGFloat = 500
    @State private var speakerTest = SpeakerTest()
    @State private var meter = Array(repeating: Float(0), count: 40)
    #if os(macOS)
    @State private var micTest = MacMicrophoneTest()
    @State private var routeError: String?
    #else
    @State private var micTest = IOSMicrophoneTest()
    #endif
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                Text("Audio test").font(CaperTheme.font(20, weight: .bold))
                    .accessibilityIdentifier("audio-preferences-sheet")
                Spacer()
                Button(action: close) {
                    CaperIcon(name: "x").frame(width: 28, height: 28)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close audio settings")
                .accessibilityIdentifier("close-audio-preferences")
                .keyboardShortcut(.cancelAction)
            }
            ScrollView {
                controls.fixedSize(horizontal: false, vertical: true)
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { controlsHeight = $0 }
            }
            .accessibilityIdentifier("audio-preferences-controls")
            #if os(macOS)
            .frame(maxHeight: min(controlsHeight, 560))
            #else
            .frame(maxHeight: min(controlsHeight, 620))
            #endif
        }.padding(22)
            .frame(maxWidth: .infinity)
            .background(CaperTheme.surface)
            #if os(iOS)
            .presentationDetents([.height(min(controlsHeight, 620) + 90)])
            #endif
            .task { await voice.refreshAudioDevices() }
            .onChange(of: voice.phase) { _, phase in
                if phase != .idle && phase != .failed { micTest.close() }
            }
            .onChange(of: voice.outputGain) { _, gain in speakerTest.setGain(gain) }
            .onDisappear { micTest.close(); speakerTest.stop() }
            .task(id: micTest.recording) {
                // Web's input meter: 40 segments, a new level every 80 ms.
                guard micTest.recording else { meter = Array(repeating: 0, count: 40); return }
                while !Task.isCancelled && micTest.recording {
                    meter = Array(meter.dropFirst()) + [min(1, sqrt(micTest.level) * 2.5)]
                    do { try await Task.sleep(for: .milliseconds(80)) } catch { return }
                }
            }
    }

    private var controls: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Only you can hear these tests.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
            #if os(macOS)
            HStack(alignment: .top, spacing: 18) { microphoneColumn; speakerColumn }
            #else
            microphoneColumn
            speakerColumn
            #endif
            microphoneTestCard
            if debugEnabled {
                DisclosureGroup("Audio diagnostics") { AudioDiagnosticsView(voice: voice) }
            }
        }
    }

    private var microphoneColumn: some View {
        VStack(alignment: .leading, spacing: 10) {
            #if os(macOS)
            CaperDevicePicker(title: "Microphone", selection: inputRoute, devices: voice.availableInputs)
                .accessibilityIdentifier("audio-input-device").disabled(micTest.recording)
            if let routeError { Text(routeError).font(CaperTheme.font(11)).foregroundStyle(.red) }
            #else
            AudioRouteRow(title: "Microphone", value: voice.availableInputs.first(where: { $0.id == voice.selectedInputID })?.name ?? "System default")
            #endif
            HStack { Text("Microphone volume"); Spacer(); Text("\(voice.inputGain)%") }.font(CaperTheme.font(12))
            CaperSlider(value: inputGain, in: 0...200, step: 1)
                .accessibilityLabel("Test microphone volume")
                .accessibilityValue("\(voice.inputGain)%")
        }.frame(maxWidth: .infinity, alignment: .leading)
    }

    private var speakerColumn: some View {
        VStack(alignment: .leading, spacing: 10) {
            #if os(macOS)
            CaperDevicePicker(title: "Speaker", selection: outputRoute, devices: voice.availableOutputs)
                .accessibilityIdentifier("audio-output-device")
            #else
            HStack {
                AudioRouteRow(title: "Speaker", value: voice.availableOutputs.first(where: { $0.id == voice.selectedOutputID })?.name ?? "System default")
                SystemAudioRoutePicker().frame(width: 44, height: 36)
                    .accessibilityLabel("Choose system audio route")
                    .accessibilityIdentifier("system-audio-route-picker")
            }
            #endif
            HStack { Text("Speaker volume"); Spacer(); Text("\(voice.outputGain)%") }.font(CaperTheme.font(12))
            CaperSlider(value: outputGain, in: 0...200, step: 1)
                .accessibilityLabel("Test speaker volume")
                .accessibilityValue("\(voice.outputGain)%")
            Button(speakerTest.playing ? "Stop speaker test" : "Test speakers") { speakerTest.toggle(voice: voice) }
                .buttonStyle(VoiceJoinButton())
                .accessibilityIdentifier("speaker-test")
                .accessibilityValue(speakerTest.playing ? "Playing" : "")
            if let error = speakerTest.error { Text(error).font(CaperTheme.font(11)).foregroundStyle(.red) }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }

    private var microphoneTestCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(micTest.recording ? "Recording…" : "Try your microphone").font(CaperTheme.font(14, weight: .bold))
                Spacer()
                if let started = micTest.startedAt {
                    TimelineView(.periodic(from: started, by: 0.1)) { timeline in
                        Text(String(format: "%.1fs", min(30, timeline.date.timeIntervalSince(started))))
                            .font(CaperTheme.font(11, weight: .bold)).monospacedDigit().foregroundStyle(CaperTheme.terracottaBright)
                    }
                }
            }
            Text("Less noise. Clearer voice.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
            VStack(alignment: .leading, spacing: 6) {
                HStack { Text("Voice enhancement"); Spacer(); Text("\(voice.voiceProcessingStrength)%") }.font(CaperTheme.font(12))
                CaperSlider(value: liveStrength, in: 0...100, step: 1)
                    .accessibilityLabel("Voice processing")
                    .accessibilityValue("\(voice.voiceProcessingStrength)%")
                    .disabled(micTest.recording)
                HStack { Text("Natural"); Spacer(); Text("Enhanced") }.font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
            }
            HStack(spacing: 14) {
                Button(micTest.recording ? "Stop recording" : "Test microphone") {
                    if micTest.recording { micTest.stopRecording() }
                    else { Task { await micTest.start(voice: voice) } }
                }
                .buttonStyle(VoiceJoinButton())
                .accessibilityIdentifier("local-mic-test")
                .disabled(recordedPreview || (voice.phase != .idle && voice.phase != .failed && voice.phase != .connected && !micTest.recording))
                VStack(alignment: .leading, spacing: 4) {
                    Text("Input level").font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
                    HStack(alignment: .center, spacing: 2) {
                        ForEach(meter.indices, id: \.self) { index in
                            Capsule().fill(micTest.recording ? CaperTheme.green : CaperTheme.border)
                                .frame(width: 3, height: 3 + CGFloat((meter[index] * 8).rounded()) * 4)
                        }
                    }.frame(height: 36)
                        .accessibilityElement().accessibilityLabel(micTest.recording ? "Received microphone level" : "Microphone level inactive")
                }
            }
            if recordedPreview { Text("TEST FIXTURE — completed local recording layout only; no microphone or playback.")
                .font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.terracottaBright) }
            if let error = micTest.error { Text(error).font(CaperTheme.font(11)).foregroundStyle(.red) }
            if micTest.hasRecording || recordedPreview {
                HStack(alignment: .top, spacing: 12) {
                    sampleCard(title: "Natural", enhanced: false)
                    sampleCard(title: "Enhanced", enhanced: true)
                }.accessibilityElement(children: .contain).accessibilityLabel("Recorded samples")
                Button("Stop playback") { micTest.stopPlayback() }.disabled(recordedPreview || micTest.playing == nil)
            }
        }.padding(14).overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
    }

    private func sampleCard(title: String, enhanced: Bool) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(CaperTheme.font(12, weight: .bold))
            Button(micTest.playing == enhanced ? "Playing…" : "Play \(title.lowercased())") { micTest.play(enhanced: enhanced) }
                .disabled(recordedPreview)
                .accessibilityLabel("Play \(title.lowercased())")
            if micTest.silent {
                Text("No audible signal detected. Check your mic and try again.").font(CaperTheme.font(11)).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }.frame(maxWidth: .infinity, alignment: .leading).padding(10)
            .background(enhanced ? CaperTheme.raised : .clear).clipShape(RoundedRectangle(cornerRadius: 6))
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
            speakerTest.stop()
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
}

/// Web's "Connection details" (Call.tsx ConnectionDiagnostics): only counters
/// this client measures; web's join timing rows need join instrumentation.
private struct ConnectionDetailsView: View {
    @Bindable var voice: VoiceClient
    let close: () -> Void
    @State private var copyStatus = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Connection details").font(CaperTheme.font(20, weight: .bold))
                Spacer()
                Button(action: close) { CaperIcon(name: "x").frame(width: 28, height: 28) }
                    .buttonStyle(.plain).accessibilityLabel("Close audio settings").keyboardShortcut(.cancelAction)
            }
            if statisticsPreview { Text("TEST FIXTURE — synthetic statistics layout; no voice connection.")
                .font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.terracottaBright) }
            if let stats = statisticsPreview ? previewStatistics : voice.diagnostics {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Self.rows(stats), id: \.0) { row in AudioRouteRow(title: row.0, value: row.1) }
                }.accessibilityElement(children: .contain).accessibilityLabel("Connection statistics")
                Text("Counters reset on reconnect.").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                HStack {
                    Button("Copy connection details") { copy(stats) }.buttonStyle(VoiceJoinButton())
                    Text(copyStatus).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                }
            } else if voice.phase == .connected {
                Text("Waiting for transport statistics…").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
            } else {
                Text("Join voice to see connection details.").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
            }
        }.padding(22).frame(maxWidth: .infinity).background(CaperTheme.surface)
            .task(id: voice.phase) {
                while !Task.isCancelled && voice.phase == .connected {
                    await voice.refreshDiagnostics()
                    do { try await Task.sleep(for: .seconds(2)) } catch { return }
                }
            }
    }

    static func rows(_ stats: VoiceDiagnostics) -> [(String, String)] {
        func megabytes(_ bytes: Int64) -> String { String(format: "%.2f MB", Double(bytes) / 1e6) }
        func kbps(_ bits: Int?) -> String { bits.map { "\(Int((Double($0) / 1_000).rounded())) kbps" } ?? "Not observed yet" }
        func ms(_ value: Int?) -> String { value.map { "\($0) ms" } ?? "Not observed yet" }
        // Join stages appear only after this client measured a join, as on Android and desktop.
        let timing: [(String, String)] = stats.timing.map { timing in [
            ("Joined", "Joined in \(timing.joinedMs) ms"),
            ("Session + publish", "\(timing.sessionMs) ms"),
            ("Transport + state", "\(timing.transportMs) ms"),
            ("Connectivity checks", stats.checks ?? "Not observed yet"),
            ("Roster", "\(timing.rosterMs) ms"),
        ] } ?? []
        return timing + [
            ("Received", megabytes(stats.receivedBytes)),
            ("Live receive", kbps(stats.receiveBitrate)),
            ("Sent", megabytes(stats.sentBytes)),
            ("Live send", kbps(stats.sendBitrate)),
            ("Packets lost", String(stats.packetsLost)),
            ("Max jitter", ms(stats.maxJitterMs)),
            ("RTT", ms(stats.roundTripMs)),
            ("Route", stats.route == "relay" ? "TURN relay" : stats.route == "direct" ? "Direct" : "Not observed yet"),
        ]
    }

    private func copy(_ stats: VoiceDiagnostics) {
        var object: [String: Any] = [
            "receivedBytes": stats.receivedBytes, "sentBytes": stats.sentBytes,
            "receiveBitrate": stats.receiveBitrate ?? 0, "sendBitrate": stats.sendBitrate ?? 0,
            "packetsLost": stats.packetsLost, "maxJitterMs": stats.maxJitterMs ?? 0,
            "roundTripMs": stats.roundTripMs ?? 0, "route": stats.route,
        ]
        if let timing = stats.timing {
            object["join"] = "Joined in \(timing.joinedMs) ms"
            object["sessionMs"] = timing.sessionMs; object["transportMs"] = timing.transportMs; object["rosterMs"] = timing.rosterMs
            if let checks = stats.checks { object["checks"] = checks }
        }
        guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys]),
              let text = String(data: data, encoding: .utf8) else { copyStatus = "Copy failed; try again."; return }
        #if os(macOS)
        NSPasteboard.general.clearContents()
        copyStatus = NSPasteboard.general.setString(text, forType: .string) ? "Copied connection details" : "Copy failed; try again."
        #else
        UIPasteboard.general.string = text
        copyStatus = "Copied connection details"
        #endif
    }

    private var statisticsPreview: Bool { CaperRuntime.isAudioPreview("audio-statistics") }
    private var previewStatistics: VoiceDiagnostics {
        VoiceDiagnostics(receivedBytes: 65_432, sentBytes: 12_345,
                         receiveBitrate: 12_800, sendBitrate: 24_000,
                         packetsLost: 3, maxJitterMs: 17, roundTripMs: 42, route: "relay",
                         checks: "4 sent · 4 answered",
                         timing: VoiceJoinTiming(joinedMs: 812, sessionMs: 214, transportMs: 391, rosterMs: 88))
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
                if voice.phase != .connected {
                    Text("No microphone capture started. Open Mic Test or join voice first.")
                        .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                }
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
