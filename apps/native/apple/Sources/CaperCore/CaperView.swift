import SwiftUI

public enum CaperTheme {
    public static let blackout = Color(red: 12/255, green: 13/255, blue: 15/255)
    public static let surface = Color(red: 21/255, green: 23/255, blue: 25/255)
    public static let border = Color(red: 52/255, green: 56/255, blue: 59/255)
    public static let text = Color(red: 243/255, green: 244/255, blue: 245/255)
    public static let terracotta = Color(red: 182/255, green: 77/255, blue: 50/255)
    public static let green = Color(red: 99/255, green: 122/255, blue: 67/255)
}

public struct CaperRootView: View {
    @State private var model: AppModel
    public init(model: AppModel = AppModel()) { _model = State(initialValue: model) }

    public var body: some View {
        Group {
            switch model.phase {
            case .loading: ProgressView("Opening Caper…")
            case .signedOut: LoginView(model: model)
            case .onboarding: ProfileView(model: model)
            case .ready: WorkspaceView(model: model)
            }
        }
        .preferredColorScheme(.dark)
        .foregroundStyle(CaperTheme.text)
        .tint(CaperTheme.terracotta)
        .background(CaperTheme.blackout.ignoresSafeArea())
        .task { if model.phase == .loading { await model.start() } }
        .alert("Caper", isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })) {
            Button("OK") { model.error = nil }
        } message: { Text(model.error ?? "") }
    }
}

private struct LoginView: View {
    @Bindable var model: AppModel
    @State private var email = ""
    @State private var code = ""
    var body: some View {
        VStack(spacing: 22) {
            Text("CAPER").font(.system(size: 34, weight: .black, design: .rounded)).tracking(5)
            Text(model.challengeID == nil ? "Sign in with your email" : "Enter the code from your email").foregroundStyle(.secondary)
            if model.challengeID == nil {
                TextField("you@example.com", text: $email).textFieldStyle(.roundedBorder)
                Button("Email me a code") { Task { await model.requestCode(email: email) } }.buttonStyle(.borderedProminent).disabled(model.busy || email.isEmpty)
            } else {
                TextField("6-digit code", text: $code).textFieldStyle(.roundedBorder)
                Button("Verify") { Task { await model.verify(code: code) } }.buttonStyle(.borderedProminent).disabled(model.busy || code.isEmpty)
                Button("Use another email") { model.challengeID = nil }
            }
            if model.busy { ProgressView() }
        }.padding(40).frame(maxWidth: 420)
    }
}

private struct ProfileView: View {
    @Bindable var model: AppModel
    @State private var username = ""
    @State private var displayName = ""
    var body: some View {
        Form {
            Text("Finish your profile").font(.title.bold())
            TextField("Username", text: $username)
            TextField("Display name", text: $displayName)
            Button("Continue") { Task { await model.saveProfile(username: username, displayName: displayName) } }.buttonStyle(.borderedProminent).disabled(model.busy || username.isEmpty || displayName.isEmpty)
        }.formStyle(.grouped).frame(maxWidth: 520)
    }
}

private struct WorkspaceView: View {
    @Bindable var model: AppModel
    var body: some View {
        NavigationSplitView {
            List(selection: $model.selectedSpaceID) {
                Section("Spaces") {
                    ForEach(model.spaces) { space in
                        Button(space.name) { Task { await model.select(space: space) } }.tag(space.id)
                    }
                }
                Section("Channels") {
                    ForEach(model.detail?.channels ?? []) { channel in
                        Button { Task { await model.select(channel: channel) } } label: {
                            Label(channel.name, systemImage: channel.private ? "lock.fill" : "number")
                        }.tag(channel.id)
                    }
                }
            }
            .scrollContentBackground(.hidden).background(CaperTheme.surface)
            .safeAreaInset(edge: .bottom) {
                HStack { Text(model.account?.displayName ?? "Caper").lineLimit(1); Spacer(); Button("Log out") { Task { await model.logout() } } }.font(.caption).padding()
            }
        } detail: {
            ChatView(model: model)
        }
    }
}

private struct ChatView: View {
    @Bindable var model: AppModel
    @Bindable var chat: ChatModel
    @Bindable var voice: VoiceClient

    init(model: AppModel) {
        self.model = model
        chat = model.chat
        voice = model.voice
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading) { Text(chat.spaceName).font(.caption).foregroundStyle(.secondary); Text("# \(chat.channelName)").font(.headline) }
                Spacer()
                Circle().fill(chat.liveState == .connected ? CaperTheme.green : .gray).frame(width: 8, height: 8)
                Text(chat.liveState == .connected ? "Live" : "Reconnecting").font(.caption)
                if ProcessInfo.processInfo.environment["CAPER_EXPERIMENTAL_VOICE"] == "1" {
                    VoiceControls(model: model, voice: voice)
                }
            }.padding().background(CaperTheme.surface).overlay(alignment: .bottom) { Divider() }
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 15) {
                        if chat.hasMore { Button("Load older messages") { Task { await chat.loadOlder() } } }
                        ForEach(chat.messages) { message in
                            VStack(alignment: .leading, spacing: 3) {
                                Text(message.author.name).font(.subheadline.bold())
                                Text(message.content.text).textSelection(.enabled)
                            }.frame(maxWidth: .infinity, alignment: .leading).id(message.id)
                        }
                    }.padding()
                }.onChange(of: chat.messages.count) { _, _ in if let id = chat.messages.last?.id { proxy.scrollTo(id, anchor: .bottom) } }
            }
            if let error = chat.error { Text(error).font(.caption).foregroundStyle(.orange).padding(.horizontal) }
            HStack(alignment: .bottom) {
                TextField("Message #\(chat.channelName)", text: $chat.draft, axis: .vertical).textFieldStyle(.roundedBorder).lineLimit(1...5)
                Button("Send") { Task { await chat.send() } }.buttonStyle(.borderedProminent).disabled(chat.sending)
            }.padding().background(CaperTheme.surface)
        }.background(CaperTheme.blackout)
    }
}

private struct VoiceControls: View {
    let model: AppModel
    @Bindable var voice: VoiceClient
    var body: some View {
        HStack {
            switch voice.phase {
            case .idle, .failed:
                Button("Join voice") { Task { await voice.join(channelID: model.selectedChannelID, name: model.account?.displayName ?? "Guest") } }.buttonStyle(.borderedProminent)
            case .joining:
                ProgressView()
                Button("Cancel") { Task { await voice.leave() } }
            case .leaving: ProgressView()
            case .connected:
                Button { Task { await voice.setMuted(!voice.muted) } } label: { Image(systemName: voice.muted ? "mic.slash.fill" : "mic.fill") }.accessibilityLabel(voice.muted ? "Unmute" : "Mute")
                Button("Leave") { Task { await voice.leave() } }
            }
        }
        .help(voice.error ?? (voice.participantNames.isEmpty ? "Voice channel" : voice.participantNames.joined(separator: ", ")))
    }
}
