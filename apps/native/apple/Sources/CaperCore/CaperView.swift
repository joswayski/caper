import QuickLook
import SwiftUI
import UniformTypeIdentifiers
#if os(iOS)
import MediaPlayer
import UIKit
#elseif os(macOS)
import AppKit
import ServiceManagement
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
    /// Primary actions are eggshell with blackout labels (shared/design.css --action);
    /// terracotta stays for selection and focus.
    public static let action = Color(red: 241/255, green: 237/255, blue: 228/255)
    public static let actionHover = Color(red: 251/255, green: 249/255, blue: 244/255)
    public static let actionPress = Color(red: 221/255, green: 215/255, blue: 203/255)
    public static let onAction = blackout
    public static let green = Color(red: 99/255, green: 122/255, blue: 67/255)
    public static let voiceSessionGreen = Color(red: 74/255, green: 168/255, blue: 107/255)
    public static let pinGold = Color(red: 228/255, green: 199/255, blue: 106/255)
    /// Web's error text (#ff9b82), in place of the system red.
    public static let error = Color(red: 1, green: 155/255, blue: 130/255)
    /// Web's muted/deafened account controls (#ED5265): the icon and its options chevron.
    public static let voiceOff = Color(red: 237/255, green: 82/255, blue: 101/255)

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
    @Environment(\.scenePhase) private var scenePhase
    public init(model: AppModel? = nil) { _model = State(initialValue: model ?? CaperRuntime.makeModel()) }

    public var body: some View {
        Group {
            switch model.phase {
            case .loading: LoadingView()
            case .signedOut: LoginPage(model: model) {}
            case .onboarding: ProfileView(model: model)
            case .ready:
                if model.restoringWorkspace { LoadingView() }
                else if model.spaces.isEmpty && model.invitations.isEmpty && model.selectedDirectMessageID == nil && model.visit == nil && !model.navigationOpen && model.spacesLoaded { FirstSpaceView(model: model) }
                else if model.spaces.isEmpty && model.invitations.isEmpty && model.selectedDirectMessageID == nil && model.visit == nil && !model.navigationOpen, let error = model.spacesError { SpacesUnavailableView(model: model, error: error) }
                else { WorkspaceView(model: model) }
            }
        }
        .preferredColorScheme(.dark)
        .foregroundStyle(CaperTheme.text)
        .tint(CaperTheme.terracottaBright)
        .buttonStyle(CaperSecondaryButton())
        .background(CaperTheme.blackout.ignoresSafeArea())
        .modifier(WhatsNewPresentation(model: model))
        .onChange(of: scenePhase, initial: true) { _, phase in model.applicationActivityChanged(active: phase == .active) }
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

/// Static placeholders keep the message layout visible without introducing motion.
private struct MessageSkeleton: View {
    var rows = 5
    var label = "Loading messages"

    var body: some View {
        VStack(spacing: 0) {
            ForEach(0..<rows, id: \.self) { index in
                HStack(alignment: .top, spacing: 10) {
                    Circle().fill(CaperTheme.border).frame(width: 34, height: 34)
                    VStack(alignment: .leading, spacing: 12) {
                        RoundedRectangle(cornerRadius: 4).fill(CaperTheme.border).frame(width: 96, height: 12)
                        RoundedRectangle(cornerRadius: 4).fill(CaperTheme.border)
                            .frame(width: index.isMultiple(of: 2) ? 180 : 128, height: 10)
                    }
                    Spacer(minLength: 0)
                }.frame(height: 50, alignment: .top).padding(.horizontal, 18).padding(.vertical, 10)
            }
        }.accessibilityElement(children: .ignore).accessibilityLabel(label)
    }
}

/// Web's first-space page: an account with no spaces names one here.
private struct FirstSpaceView: View {
    @Bindable var model: AppModel
    @State private var name = ""
    @State private var error: String?
    @State private var pending = false
    @State private var joining = false
    /// A link opened from another app, shown in the join dialog.
    @State private var opened: OpenedInviteLink?
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
                    Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Divider().overlay(CaperTheme.border)
                HStack {
                    Text("Have an invite link?").font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                    Spacer()
                    Button("Join with invite link") { opened = nil; joining = true }.buttonStyle(CaperSecondaryButton(minHeight: 42))
                        .disabled(pending).accessibilityIdentifier("first-space-join-invite")
                }
                HStack {
                    Spacer()
                    Button("Log out") { Task { await model.logout() } }.buttonStyle(.plain)
                        .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).modifier(ControlHover())
                }
            }.padding(28).frame(maxWidth: 440).frame(maxWidth: .infinity)
        }.background(CaperTheme.blackout)
            .sheet(isPresented: $joining) {
                JoinInviteSheet(model: model, opened: opened, close: { joining = false })
                    .id(opened?.id)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                    .background(CaperTheme.surface)
                    #if os(iOS)
                    .presentationDetents([.medium, .large])
                    #else
                    .frame(width: 520)
                    .frame(minHeight: 300)
                    #endif
            }
            .onChange(of: model.deliverableInviteLink?.id, initial: true) { _, _ in
                guard let link = model.takeInviteLink() else { return }
                opened = link; joining = true
            }
    }
    private func create() {
        guard !pending, model.canCreateSpace, !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        pending = true; error = nil
        Task {
            // The name stays in the field when creation fails, like web.
            do { try await model.createSpace(name: name) } catch { self.error = FriendlyError.message(for: error) }
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
            Text(error).font(CaperTheme.font(13)).foregroundStyle(CaperTheme.error).multilineTextAlignment(.center)
            Button(model.busy ? "Trying…" : "Try again") { Task { await model.loadSpaces() } }
                .buttonStyle(CaperSecondaryButton()).disabled(model.busy)
            Button("Direct messages") { model.navigationOpen = true }
            Button("Log out") { Task { await model.logout() } }.buttonStyle(.plain)
                .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).modifier(ControlHover())
        }.padding(28).frame(maxWidth: .infinity, maxHeight: .infinity).background(CaperTheme.blackout)
    }
}

private struct Wordmark: View {
    var body: some View {
        Image("CaperWordmark").renderingMode(.original).resizable().scaledToFit()
            .frame(width: 132, height: 35)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Caper")
    }
}

// Asset catalogs are generated from the same pinned Lucide vectors as the web.
struct CaperIcon: View {
    let name: String
    var size: CGFloat = 16
    var body: some View {
        Image("caper-\(name)").renderingMode(.template).resizable()
            .frame(width: size, height: size).accessibilityHidden(true)
    }
}

/// A `Label` with a Lucide icon in place of an SF Symbol, for action lists.
struct CaperLabel: View {
    let title: String
    let icon: String
    var size: CGFloat = 18
    init(_ title: String, icon: String, size: CGFloat = 18) {
        self.title = title
        self.icon = icon
        self.size = size
    }
    var body: some View {
        Label { Text(title) } icon: { CaperIcon(name: icon, size: size) }
    }
}

/// A menu item's Lucide icon. Menus take a plain image (no frame), drawn at
/// the asset's own 24pt size and tinted like the system's symbols.
func caperMenuIcon(_ name: String) -> Image {
    Image("caper-\(name)").renderingMode(.template)
}

/// A view that slides open on phones, following a finger.
protocol SlideDrawer: AnyObject {
    /// While a finger drags: how far it is open, from 0 (closed) to 1 (open).
    var drag: CGFloat? { get set }
    /// How far it moves between closed and open.
    var travel: CGFloat { get }
    /// The slide to rest under way, so a finger that lands during it picks the
    /// view up where it is on screen instead of where it is headed.
    var settle: DrawerSettle? { get set }
    /// The resting place it was last drawn for, 0 or 1.
    var rest: CGFloat? { get set }
    /// The finger's direction to open it: right (1) or left (-1).
    static var opening: CGFloat { get }
}

extension SlideDrawer {
    /// How far open it is on screen, or nil once it is at rest.
    var inFlight: CGFloat? {
        guard let settle, !settle.finished() else { return nil }
        return settle.value()
    }
    /// Records a slide that a button starts (a swipe records its own release),
    /// from wherever the view is now.
    func slide(to target: CGFloat, timing: DrawerSettle.Timing, reduceMotion: Bool) {
        guard drag == nil, settle?.to != target else { return }
        settle = reduceMotion ? nil : DrawerSettle(from: inFlight ?? 1 - target, to: target, timing: timing)
    }
    /// Where it is drawn at `date`, from 0 (closed) to 1 (open): under the
    /// finger, along its slide, or at `rest`. A rest that changed with no slide
    /// recorded for it (a model change) slides there from where it was.
    func position(rest: CGFloat, fallback: DrawerSettle.Timing, reduceMotion: Bool, at date: Date = Date()) -> CGFloat {
        if let drag { return drag }
        if let previous = self.rest, previous != rest, settle?.to != rest {
            let from = settle.flatMap { $0.finished(at: date) ? nil : $0.value(at: date) } ?? previous
            settle = reduceMotion ? nil : DrawerSettle(from: from, to: rest, start: date, timing: fallback)
        }
        self.rest = rest
        guard let settle, settle.to == rest, !settle.finished(at: date) else { return rest }
        return settle.value(at: date)
    }
}

/// A drawer slide's path and timing. Slides use one cubic curve with the web's
/// `SETTLE` easing. Each frame draws the drawer where this curve is
/// (`DrawerPose`), so a finger catching a moving view starts exactly where it
/// is drawn, and no two slides can add up past either end.
struct DrawerSettle {
    struct Timing {
        let duration: Double
        let curve: UnitCurve
        /// Web `SETTLE`: cubic-bezier(0.32, 0.72, 0, 1).
        static let easing = UnitCurve.bezier(startControlPoint: UnitPoint(x: 0.32, y: 0.72), endControlPoint: UnitPoint(x: 0, y: 1))
        /// Settling back after a cancelled drag.
        static let settle = Timing(duration: 0.32, curve: easing)
        /// Buttons, Back and channel choices (web `TAP`), which have no finger
        /// speed to carry on: it leaves at once and slows steadily, rather than
        /// leaping most of the way in two frames and then crawling into place.
        static let tap = Timing(duration: 0.30, curve: .bezier(startControlPoint: UnitPoint(x: 0.33, y: 1),
                                                              endControlPoint: UnitPoint(x: 0.68, y: 1)))
        /// A released swipe (web `releaseTiming`).
        static func release(_ release: DrawerRelease) -> Timing {
            Timing(duration: release.duration, curve: .bezier(startControlPoint: UnitPoint(x: 0.2, y: 0.2 * release.slope),
                                                               endControlPoint: UnitPoint(x: 0.35, y: 1)))
        }
        var animation: Animation { .timingCurve(curve, duration: duration) }
    }
    let from: CGFloat
    let to: CGFloat
    let start: Date
    let timing: Timing

    init(from: CGFloat, to: CGFloat, start: Date = Date(), timing: Timing) {
        self.from = from
        self.to = to
        self.start = start
        self.timing = timing
    }

    func value(at date: Date = Date()) -> CGFloat {
        guard timing.duration > 0 else { return to }
        let progress = min(1, max(0, date.timeIntervalSince(start) / timing.duration))
        return from + (to - from) * CGFloat(timing.curve.value(at: progress))
    }
    func finished(at date: Date = Date()) -> Bool { date.timeIntervalSince(start) >= timing.duration }
}

/// Where a released drawer swipe goes and how fast, as web `releaseTiming`:
/// past halfway, or flung 40 points toward the other view, it switches; a
/// held drag gets the full finish, and a fling carries its bounded speed into
/// a curve that cannot overshoot.
struct DrawerRelease: Equatable {
    /// Switches to the other view.
    let commit: Bool
    /// How far open it ends, 0 or 1.
    let target: CGFloat
    let duration: Double
    /// The curve's initial slope: the release speed over the average speed, up to 3.
    let slope: Double

    /// - Parameters:
    ///   - open: The swipe started on the open drawer.
    ///   - position: How far open it is, from 0 to 1.
    ///   - velocity: The finger's speed toward open, in points per second.
    ///   - distance: How far the finger moved toward open, in points.
    ///   - travel: The drawer's travel, in points.
    init(open: Bool, position: CGFloat, velocity: CGFloat, distance: CGFloat, travel: CGFloat) {
        // Toward the other view, in points per second.
        let toward = open ? -velocity : velocity
        let travelled = abs(position - (open ? 1 : 0))
        commit = toward > -300 && (travelled >= 0.5 || (toward >= 300 && abs(distance) >= 40))
        target = commit != open ? 1 : 0
        let remaining = target - position
        let left = Double(abs(remaining))
        // Toward the target, in travels per second.
        let speed = Double((remaining > 0 ? 1 : -1) * velocity / max(travel, 1))
        duration = speed > 0 ? min(0.32, max(0.16, 3 * left / speed)) : 0.32
        slope = left > 0 ? min(3, max(0, speed * duration / left)) : 0
    }
}

/// Phones show the conversation or Browse (spaces, channels and the account
/// bar), never both. The conversation, a layer above the whole of Browse, slides
/// fully off the still Browse, following a finger from anywhere on its surface.
/// Buttons, Back and channel choices animate the same slide.
@Observable private final class BrowseDrawer: SlideDrawer {
    /// While a finger drags: how much of Browse is uncovered, from 0 (conversation) to 1 (Browse).
    var drag: CGFloat?
    /// How far the conversation moves: the full width and its edge shadow.
    var travel: CGFloat = 1
    @ObservationIgnored var settle: DrawerSettle?
    @ObservationIgnored var rest: CGFloat?
    static let opening: CGFloat = 1
    /// While a released swipe navigates, which already set its speed-matched curve.
    @ObservationIgnored var releasing = false
}

/// Marks a Browse/conversation switch. Only the conversation layer moves with
/// it: nothing inside animates on its own, so no control lags behind the slide.
private struct BrowseSlideKey: TransactionKey { static let defaultValue = false }
private extension Transaction {
    var browseSlide: Bool {
        get { self[BrowseSlideKey.self] }
        set { self[BrowseSlideKey.self] = newValue }
    }
}

/// The member list on phones: a panel that slides in from the trailing edge
/// over the conversation with Browse's curve, following a finger the same
/// way. It opens from the conversation and closes from the panel or backdrop.
@Observable private final class MembersDrawer: SlideDrawer {
    /// While a finger drags: how much of the panel is in view, from 0 to 1.
    var drag: CGFloat?
    /// How far the panel moves: its width and margin, plus room for its shadow.
    var travel: CGFloat = 1
    @ObservationIgnored var settle: DrawerSettle?
    @ObservationIgnored var rest: CGFloat?
    static let opening: CGFloat = -1
}

/// Clips sideways only, so backgrounds still reach under the status bar and
/// home indicator.
private struct SideClip: Shape {
    func path(in rect: CGRect) -> Path { Path(rect.insetBy(dx: 0, dy: -rect.height)) }
}

/// Draws a drawer's layer where it is on each frame: under the finger, along
/// its recorded slide, or at rest. SwiftUI's own curves add up where they
/// overlap: a release's drag and navigation changes each animated the layer,
/// which could draw it past its rest, on the far side, before it eased back.
/// One recorded curve, evaluated per frame, stays between its ends.
private struct DrawerPose<Drawer: SlideDrawer & Observable>: ViewModifier {
    /// Where it rests: 1 open, 0 closed.
    let rest: CGFloat
    /// The slide for a model change, which records none of its own.
    let fallback: DrawerSettle.Timing
    /// The layer's offset and opacity at a position.
    let pose: (CGFloat) -> (offset: CGSize, opacity: Double)
    @Environment(Drawer.self) private var drawer: Drawer?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// Bumped as a slide ends, which stops the frame clock.
    @State private var settled = 0

    func body(content: Content) -> some View {
        let now = Date()
        let current = position(at: now)
        let settle = drawer?.drag == nil ? drawer?.settle : nil
        let remaining = settle.map { $0.timing.duration - now.timeIntervalSince($0.start) } ?? 0
        let sliding = remaining > 0
        let _ = settled
        TimelineView(.animation(paused: !sliding)) { context in
            let drawn = pose(sliding ? position(at: context.date) : current)
            // Nothing else animates the pose: it is already where it belongs this frame.
            content.transaction { $0.animation = nil } body: { $0.offset(drawn.offset).opacity(drawn.opacity) }
        }
        .task(id: sliding ? settle?.start : nil) {
            guard sliding else { return }
            try? await Task.sleep(for: .seconds(remaining))
            if !Task.isCancelled { settled += 1 }
        }
    }

    private func position(at date: Date) -> CGFloat {
        drawer?.position(rest: rest, fallback: fallback, reduceMotion: reduceMotion, at: date) ?? rest
    }
}

/// Poses the conversation for the slide on phones; Browse beneath it stays
/// still. Only this reads the drag, so a moving finger re-renders the pose
/// rather than the layer itself.
private struct BrowseLayer: ViewModifier {
    let showsBrowse: Bool
    /// The layer's offset for how much of Browse is uncovered.
    let pose: (CGFloat) -> CGSize

    func body(content: Content) -> some View {
        let pose = self.pose
        // Nothing inside animates with the switch, so no control lags behind the slide.
        content.transaction { transaction in
            if transaction.browseSlide { transaction.animation = nil }
        }
        .modifier(DrawerPose<BrowseDrawer>(rest: showsBrowse ? 1 : 0, fallback: .tap) { (pose($0), 1) })
    }
}

/// Lifting a drawer swipe also ends the press on the control under the finger,
/// which then taps it: the view moves with the finger, so it lifts on the same
/// image, avatar, reaction or row it started on. SwiftUI can't cancel a press
/// that has started, so controls on swiping surfaces skip a tap or long press
/// that lands while a swipe moves or just after it lifts, as the web suppresses
/// a drag's release click. It is only a time window, renewed by every move, so
/// nothing a lost gesture leaves behind can keep controls off.
@MainActor enum SwipeTapGuard {
    private static var quietUntil = Date.distantPast
    /// A swipe moved with the finger. A held finger still reports small moves.
    static func follow() { quiet(for: 0.35) }
    /// The finger lifted, or the system took the touch. The lift's own tap
    /// arrives with it, in the same touch event.
    static func release() { quiet(for: 0.12) }
    private static func quiet(for seconds: Double) {
        quietUntil = max(quietUntil, Date().addingTimeInterval(seconds))
    }
    /// A control was "tapped" by a swipe.
    static var swiping: Bool { Date() < quietUntil }
    /// `action`, skipped when a swipe's lift triggered it.
    static func unlessSwiping(_ action: @escaping @MainActor () -> Void) -> @MainActor () -> Void {
        { if !swiping { action() } }
    }
}

/// A message's links skip a swipe's lift (the text moves with the finger, so
/// it lifts on the link it started on) and otherwise open as they would have.
private struct SwipeGuardedLinks: ViewModifier {
    @Environment(\.openURL) private var openURL
    func body(content: Content) -> some View {
        let openURL = self.openURL
        content.environment(\.openURL, OpenURLAction { url in
            if !SwipeTapGuard.swiping { openURL(url) }
            return .handled
        })
    }
}

private typealias BrowseSwipe = DrawerSwipe<BrowseDrawer>
private typealias MembersSwipe = DrawerSwipe<MembersDrawer>

// Horizontal swipes start across the content surface. Attach outside text entry
// and audio controls so those keep their own selection and slider gestures.
// The whole view takes touches even while disabled, so attach it only where
// the view itself would take them.
private struct DrawerSwipe<Drawer: SlideDrawer & Observable>: ViewModifier {
    let open: Bool
    let enabled: Bool
    var excludedStart: CGRect = .null
    let navigate: () -> Void
    @Environment(Drawer.self) private var drawer: Drawer?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// Undecided until the first move; then following the finger, or not this gesture's.
    @State private var tracking: Bool?
    /// Where the drawer was and the finger's offset (toward open) when it started following.
    @State private var grab: (position: CGFloat, offset: CGFloat)?
    /// The gesture this state belongs to, by where it started, and when it
    /// ended. After an end, or from another start, a move begins a new gesture
    /// decided afresh, so no earlier gesture can leave a stale slide. Moves that
    /// keep an ended gesture's start and arrive far along are its tail: following
    /// them reopened the slide it had just finished. A new drag from that same
    /// point reports its first move just past the minimum distance. From another
    /// start a far first move is still new: a link or button under the finger
    /// holds back a fast swipe's first report until it gives up the touch.
    @State private var gesture: (start: CGPoint, endedAt: Date?)?
    @GestureState private var dragging = false

    func body(content: Content) -> some View {
        let opening = Drawer.opening
        // Global coordinates: the view itself moves with the finger. The list
        // stops scrolling while it follows a slide, and only while the gesture
        // lasts. Turning off hit-testing mid-swipe let the system cancel it over
        // a scrolling list, so it slid back; controls skip the lift's tap instead
        // (`SwipeTapGuard`).
        content.contentShape(Rectangle()).scrollDisabled(dragging && tracking == true)
            .simultaneousGesture(DragGesture(minimumDistance: 12, coordinateSpace: .global)
                .updating($dragging) { _, active, _ in active = true }
                .onChanged { value in
                    let sameStart = gesture?.start == value.startLocation
                    if !sameStart || gesture?.endedAt != nil {
                        // An ended gesture's tail keeps its start and arrives far along.
                        if sameStart { guard hypot(value.translation.width, value.translation.height) < 60 else { return } }
                        // An abandoned slide of an earlier gesture settles back first.
                        if tracking == true, let drawer, let position = drawer.drag {
                            drawer.settle = reduceMotion ? nil : DrawerSettle(from: position, to: open ? 1 : 0, timing: .settle)
                            drawer.drag = nil
                        }
                        gesture = (value.startLocation, nil)
                        tracking = nil
                        grab = nil
                    }
                    // Toward open, in points.
                    let dx = opening * value.translation.width, dy = value.translation.height
                    if tracking == nil {
                        tracking = enabled && drawer != nil && !excludedStart.contains(value.startLocation) && abs(dx) >= abs(dy) * 1.5 && (open ? dx < 0 : dx > 0)
                        guard tracking == true, let drawer else { return }
                        // Catch a view still sliding where it is drawn, and follow
                        // from here rather than jumping by the gesture's start distance.
                        grab = (drawer.inFlight ?? (open ? 1 : 0), dx)
                        drawer.settle = nil
                    }
                    guard tracking == true, let drawer, let grab else { return }
                    SwipeTapGuard.follow()
                    drawer.drag = min(1, max(0, grab.position + (dx - grab.offset) / drawer.travel))
                }
                .onEnded { value in
                    // Only the current gesture's end acts; an earlier one's leaves it alone.
                    guard gesture?.start == value.startLocation else { return }
                    let followed = tracking == true && gesture?.endedAt == nil
                    defer { tracking = nil; grab = nil; if gesture?.endedAt == nil { gesture?.endedAt = Date() } }
                    guard followed, let drawer, let position = drawer.drag else { return }
                    let release = DrawerRelease(open: open, position: position, velocity: opening * value.velocity.width,
                                                distance: opening * value.translation.width, travel: drawer.travel)
                    let timing = DrawerSettle.Timing.release(release)
                    drawer.settle = reduceMotion ? nil : DrawerSettle(from: position, to: release.target, timing: timing)
                    SwipeTapGuard.release()
                    withAnimation(reduceMotion ? nil : timing.animation) {
                        drawer.drag = nil
                        if release.commit {
                            // Only Browse marks its switch; the member list animates inside the conversation.
                            if let browse = drawer as? BrowseDrawer {
                                browse.releasing = true
                                withTransaction(\.browseSlide, true, navigate)
                                browse.releasing = false
                            } else { navigate() }
                        }
                    }
                }, including: enabled ? .all : .subviews)
            .onChange(of: dragging) { _, active in
                guard !active else { return }
                // After onEnded, if it runs: whatever ended the gesture, nothing of it
                // stays. A cancelled drag (by a scroll, or the system) slides back.
                // Only for that gesture: a newer one that has begun meanwhile keeps its state.
                let ended = gesture?.start
                DispatchQueue.main.async {
                    guard !dragging, gesture?.start == ended else { return }
                    let followed = tracking == true
                    tracking = nil
                    grab = nil
                    if gesture?.endedAt == nil { gesture?.endedAt = Date() }
                    guard followed, let drawer, let position = drawer.drag else { return }
                    let timing = DrawerSettle.Timing.settle
                    drawer.settle = reduceMotion ? nil : DrawerSettle(from: position, to: open ? 1 : 0, timing: timing)
                    SwipeTapGuard.release()
                    withAnimation(reduceMotion ? nil : timing.animation) { drawer.drag = nil }
                }
            }
    }
}

private enum WorkspaceSheet: Identifiable {
    /// `joinInvite` carries a link opened from another app, or nil.
    case login, profile, createSpace, joinInvite(OpenedInviteLink?), createChannel, newDirectMessage, manageSpace, manageChannel(Channel), invitation(Space), channelInvitation(ChannelInvitation), leaveSpace, audio, connection, diagnostics, privacy, notifications
    var id: String {
        switch self {
        case .login: "login"
        case .profile: "profile"
        case .createSpace: "create-space"
        case .joinInvite(let opened): opened.map { "join-invite-\($0.id)" } ?? "join-invite"
        case .createChannel: "create-channel"
        case .newDirectMessage: "new-direct-message"
        case .manageSpace: "manage-space"
        case .manageChannel(let channel): "manage-\(channel.id)"
        case .invitation(let space): "invitation-\(space.id)"
        case .channelInvitation(let invitation): "channel-invitation-\(invitation.id)"
        case .leaveSpace: "leave-space"
        case .audio: "audio"
        case .connection: "connection"
        case .diagnostics: "diagnostics"
        case .privacy: "privacy"
        case .notifications: "notifications"
        }
    }
}

/// A workspace dialog's height: its content's, up to `maximum`, where the
/// dialog's own scroll view takes over. Short dialogs no longer sit in a
/// mostly empty full-height panel.
private struct DialogHeight: Layout {
    let maximum: CGFloat
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? subviews.first?.sizeThatFits(.unspecified).width ?? 0
        guard let content = subviews.first else { return CGSize(width: width, height: 0) }
        let ideal = content.sizeThatFits(ProposedViewSize(width: width, height: nil)).height
        return CGSize(width: width, height: min(maximum, ideal))
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for subview in subviews {
            subview.place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(bounds.size))
        }
    }
}

private struct WorkspaceView: View {
    @Bindable var model: AppModel
    @State private var sheet: WorkspaceSheet?
    @AppStorage("caper.channelSidebarWidth") private var sidebarWidth = 280.0
    @State private var sidebarDragStart: Double?
    /// The width while a drag resizes the sidebar; saved once when the drag ends.
    @State private var sidebarDragWidth: Double?
    private var liveSidebarWidth: Double { sidebarDragWidth ?? sidebarWidth }
    @FocusState private var sidebarFocused: Bool
    @State private var membersPreference: Bool?
    @State private var modalDismissDisabled = false
    @State private var browseDrawer = BrowseDrawer()
    @State private var membersDrawer = MembersDrawer()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private let parityFixture: String?
    private let parityMode: Bool

    init(model: AppModel) {
        self.model = model
        let environment = ProcessInfo.processInfo.environment
        parityMode = environment["CAPER_TEST_MODE"] == "parity"
        parityFixture = environment["CAPER_TEST_MODE"] == "parity" ? environment["CAPER_UI_FIXTURE"] : nil
        _sheet = State(initialValue: parityFixture == "login" ? .login : nil)
    }

    var body: some View {
        workspace
    }

    private var workspaceContent: some View {
        GeometryReader { geometry in
            let narrow = geometry.size.width <= 760
            let membersVisible = membersPreference ?? !narrow
            VStack(spacing: 0) {
                if let error = model.navigationError {
                    HStack {
                        Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                        Spacer()
                        Button("Retry opening") { Task { await model.retryNavigation() } }
                            .disabled(model.openingSpaceID != nil)
                    }.padding(12).background(CaperTheme.surface)
                }
                // Phones slide the conversation fully over Browse, account bar and
                // all; wider windows show both.
                let conversationAvailable = !model.spaces.isEmpty || model.selectedDirectMessageID != nil || model.visit != nil
                let showsBrowse = !narrow || model.navigationOpen || !conversationAvailable
                // Past the width, so the edge shadow leaves too.
                let travel = geometry.size.width + 48
                ZStack(alignment: .leading) {
                    VStack(spacing: 0) {
                        HStack(spacing: 0) {
                            SpaceRail(model: model, narrow: narrow, showLogin: { sheet = .login }, create: { sheet = .createSpace }, openInvitation: { sheet = .invitation($0) })
                                .frame(width: 60)
                                .modifier(BrowseSwipe(open: true, enabled: narrow && sheet == nil && (model.selectedChannel != nil || model.selectedDirectMessageID != nil || model.visit != nil), navigate: { slide(open: false) }))
                            ChannelSidebar(
                                model: model,
                                sheet: $sheet,
                                narrow: narrow,
                                close: { slide(open: false) }
                            )
                            .frame(width: narrow ? nil : CGFloat(min(liveSidebarWidth, sidebarMaximum(for: geometry.size.width))))
                            .frame(maxWidth: narrow ? .infinity : CGFloat(min(liveSidebarWidth, sidebarMaximum(for: geometry.size.width))))
                            .clipShape(RoundedRectangle(cornerRadius: narrow ? 16 : 0))
                            .padding(.top, narrow ? 8 : 0)
                            .padding(.trailing, narrow ? 8 : 0)
                            .overlay(alignment: .trailing) {
                                if !narrow {
                                    Color.clear
                                        .frame(width: 8).frame(maxHeight: .infinity)
                                        .contentShape(Rectangle())
                                        .modifier(ResizeCursor())
                                        .highPriorityGesture(DragGesture(minimumDistance: 1, coordinateSpace: .global).onChanged { value in
                                            sidebarFocused = true
                                            if sidebarDragStart == nil { sidebarDragStart = liveSidebarWidth }
                                            // Follow the pointer without writing preferences on every move.
                                            sidebarDragWidth = clampedSidebarWidth((sidebarDragStart ?? liveSidebarWidth) + Double(value.translation.width), viewport: geometry.size.width)
                                        }.onEnded { _ in
                                            if let width = sidebarDragWidth { sidebarWidth = width }
                                            sidebarDragWidth = nil; sidebarDragStart = nil
                                        })
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
                                                get: { min(liveSidebarWidth, sidebarMaximum(for: geometry.size.width)) },
                                                set: { resizeSidebar($0, viewport: geometry.size.width) }
                                            ), in: 220...max(230, sidebarMaximum(for: geometry.size.width)), step: 10)
                                        }
                                        .accessibilityLabel("Channel sidebar width")
                                        .accessibilityIdentifier("channel-sidebar-resize")
                                        .accessibilityValue("\(Int(min(liveSidebarWidth, sidebarMaximum(for: geometry.size.width)))) pixels")
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
                                    if geometry.size.width - 60 - min(liveSidebarWidth, sidebarMaximum(for: geometry.size.width)) >= 540 {
                                        HStack(spacing: 0) {
                                            ConversationStage(model: model, narrow: false, browse: { slide(open: true) }, createChannel: { sheet = .createChannel }, membersVisible: membersVisible) {
                                                showMembers(!membersVisible)
                                            }
                                            if membersVisible && model.selectedDirectMessageID == nil && model.visit == nil { MemberPresenceView(model: model, close: { showMembers(false) }).frame(width: 220) }
                                        }
                                    } else {
                                        ZStack(alignment: .trailing) {
                                            ConversationStage(model: model, narrow: false, browse: { slide(open: true) }, createChannel: { sheet = .createChannel }, membersVisible: membersVisible) {
                                                showMembers(!membersVisible)
                                            }
                                            if membersVisible && model.selectedDirectMessageID == nil && model.visit == nil { MemberPresenceView(model: model, close: { showMembers(false) }).frame(width: 220).padding(.top, 50) }
                                        }
                                    }
                                }
                            }
                        }
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        // Phones keep the account bar in Browse, full width beneath the
                        // channels: the conversation slides off to reveal it.
                        if narrow { AccountBar(model: model, sheet: $sheet) }
                    }
                    .allowsHitTesting(showsBrowse)
                    .accessibilityElement(children: .contain)
                    .accessibilityHidden(!showsBrowse)
                    if narrow && conversationAvailable {
                        ZStack(alignment: .trailing) {
                            ConversationStage(model: model, narrow: true, browse: { slide(open: true) }, createChannel: { sheet = .createChannel }, membersVisible: membersVisible) {
                                showMembers(!membersVisible)
                            }
                            if model.selectedDirectMessageID == nil && model.visit == nil {
                                MembersOverlay(model: model, shows: membersVisible && !model.previewingChannel,
                                               width: min(280, geometry.size.width - 64)) { showMembers(false) }
                            }
                        }
                        // Returning from Browse must not reopen the keyboard on its own.
                        .environment(\.composerCovered, showsBrowse)
                        // Full height, under the status bar and home indicator too, with a
                        // soft shadow cast from the conversation's edge onto Browse, like the web's.
                        .background {
                            Rectangle().fill(CaperTheme.blackout)
                                .shadow(color: .black.opacity(0.45), radius: 16, x: -12)
                                .ignoresSafeArea(.container, edges: .vertical)
                        }
                        .modifier(BrowseLayer(showsBrowse: showsBrowse) { CGSize(width: $0 * travel, height: 0) })
                        .allowsHitTesting(!showsBrowse)
                        .accessibilityElement(children: .contain)
                        .accessibilityHidden(showsBrowse)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .clipShape(SideClip())
                .onChange(of: travel, initial: true) { _, travel in browseDrawer.travel = max(travel, 1) }
                .onChange(of: narrow, initial: true) { _, narrow in model.narrowLayout = narrow }
                // A model change (a channel that finished opening) slides too:
                // `DrawerPose` records it, so a finger can catch the conversation mid-slide.
                .onChange(of: showsBrowse) { endEditing() }
            }
            #if os(iOS)
            // The reader already measures the keyboard-avoiding viewport. Keep
            // keyboard resizing at that boundary, not in each transformed layer.
            .frame(width: narrow ? geometry.size.width : nil, height: narrow ? geometry.size.height : nil, alignment: .topLeading)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: narrow ? .topLeading : .center)
            .ignoresSafeArea(narrow ? .keyboard : [], edges: .bottom)
            #else
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            #endif
            .background(CaperTheme.blackout)
            .environment(browseDrawer)
            .environment(membersDrawer)
        }
    }

    /// Buttons switch with the steady tap slide. A released swipe keeps the
    /// curve it set to match the finger's speed.
    private func slide(open: Bool) {
        if browseDrawer.releasing { model.navigationOpen = open; return }
        guard model.navigationOpen != open else { return }
        browseDrawer.slide(to: open ? 1 : 0, timing: .tap, reduceMotion: reduceMotion)
        var transaction = Transaction(animation: DrawerSettle.Timing.tap.animation)
        transaction.browseSlide = true
        withTransaction(transaction) { model.navigationOpen = open }
    }

    /// Opens or closes the member list. Recorded first, so a closing phone
    /// panel keeps its list in view until it has slid away.
    private func showMembers(_ visible: Bool) {
        guard membersPreference != visible else { return }
        membersDrawer.slide(to: visible ? 1 : 0, timing: .tap, reduceMotion: reduceMotion)
        membersPreference = visible
    }

    /// The hidden view stays in place to slide back, so it must not keep the keyboard.
    private func endEditing() {
        #if os(iOS)
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        #elseif os(macOS)
        NSApp.keyWindow?.makeFirstResponder(nil)
        #endif
    }

    private var workspace: some View {
        ZStack {
            workspaceContent
                .disabled(modalSheet.wrappedValue != nil)
                // Hide this container without overriding its children's own
                // hidden states when the workspace becomes accessible again.
                .accessibilityElement(children: .contain)
                .accessibilityHidden(modalSheet.wrappedValue != nil)
            // A sibling, not an overlay on disabled content: modal controls must
            // retain their own hit-testing, keyboard and accessibility environment.
            if let item = modalSheet.wrappedValue {
                GeometryReader { geometry in
                    // As on web, a dialog fits its content and hangs from a fixed top
                    // edge, so loading, errors and save bars only extend it downward.
                    let top = max(16, geometry.size.height * 0.1)
                    ZStack(alignment: .top) {
                        Color.black.opacity(0.55).ignoresSafeArea()
                            .onTapGesture { if !modalDismissDisabled { sheet = nil } }
                            .accessibilityHidden(true)
                            .modifier(Entrance.scrim)
                        DialogHeight(maximum: min(680, max(1, geometry.size.height - top - 16))) {
                            // A request finishing after its dialog was dismissed must
                            // not close the dialog that replaced it.
                            WorkspaceSheetView(item: item, model: model, show: { sheet = $0 }) { if sheet?.id == item.id { sheet = nil } }
                                .id(item.id)
                        }
                            .frame(width: min(item.id.contains("manage") ? 600 : 560, geometry.size.width - 32))
                            .background(CaperTheme.surface.onTapGesture {})
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                            .shadow(color: .black.opacity(0.3), radius: 24, y: 12)
                            .modifier(Entrance.dialog)
                            .padding(.top, top)
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
        // Timeline, thread and (on the Mac) pinned attachments open here, over the whole window.
        .modifier(MediaViewerHost())
        .onChange(of: sheet?.id, initial: true) { _, new in
            modalDismissDisabled = false
            model.whatsNew.workspaceModalOpen = new != nil
        }
        .onDisappear { model.whatsNew.workspaceModalOpen = false }
        // A link opened from another app replaces any open dialog.
        .onChange(of: model.deliverableInviteLink?.id, initial: true) { _, _ in
            if let link = model.takeInviteLink() { sheet = .joinInvite(link) }
        }
        .onChange(of: model.notificationInvitation, initial: true) { _, invitation in
            if let invitation { sheet = .invitation(invitation); model.notificationInvitation = nil }
        }
        .onChange(of: model.notificationChannelInvitation, initial: true) { _, invitation in
            if let invitation { sheet = .channelInvitation(invitation); model.notificationChannelInvitation = nil }
        }
        .task(id: model.detail?.space.id) {
            guard sheet == nil, model.detail != nil else { return }
            if parityFixture == "manage-space" || parityFixture == "modal" { sheet = .manageSpace }
            else if parityFixture == "manage-channel", let channel = model.detail?.channels.first(where: { $0.private }) { sheet = .manageChannel(channel) }
            else if parityFixture == "new-direct-message" { sheet = .newDirectMessage }
            else if parityFixture == "voice-roster" { CaperRuntime.showVoiceRosterPreview(model) }
            else if parityFixture == "whats-new" { CaperRuntime.showWhatsNewPreview(model) }
        }
        #if os(iOS)
        .overlay(alignment: .topLeading) { if parityMode { ParityPasteboardProbe() } }
        #endif
    }

    private func sidebarMaximum(for viewport: CGFloat) -> Double {
        max(220, min(440, Double(viewport) - 60 - 320))
    }

    private func clampedSidebarWidth(_ value: Double, viewport: CGFloat) -> Double {
        min(sidebarMaximum(for: viewport), max(220, value.rounded()))
    }

    private func resizeSidebar(_ value: Double, viewport: CGFloat) {
        sidebarWidth = clampedSidebarWidth(value, viewport: viewport)
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
        // A classic (always shown) scroller would squeeze the tiles off-centre in the 60pt rail.
        ScrollView(.vertical, showsIndicators: false) {
            VStack(spacing: 10) {
                ForEach(model.spaces) { space in
                    SpaceRailButton(model: model, space: space, narrow: narrow)
                }
                if !model.invitations.isEmpty {
                    Text("INVITES").font(CaperTheme.font(8, weight: .bold)).foregroundStyle(CaperTheme.muted)
                        .accessibilityLabel("Pending invitations")
                    ForEach(model.invitations) { invitation in
                        Button { if !SwipeTapGuard.swiping { openInvitation(invitation) } } label: {
                            Text(String(invitation.name.prefix(1)).uppercased()).font(CaperTheme.font(13, weight: .black))
                                .frame(width: 40, height: 40).background(CaperTheme.surface)
                                .clipShape(RoundedRectangle(cornerRadius: 12))
                                .overlay(RoundedRectangle(cornerRadius: 12).stroke(CaperTheme.terracottaBright, style: StrokeStyle(lineWidth: 1, dash: [3])))
                        }.buttonStyle(.plain).modifier(ControlHover()).help("Invitation to \(invitation.name)").accessibilityLabel("Invitation to \(invitation.name)")
                    }
                }
                Button(action: SwipeTapGuard.unlessSwiping(model.account == nil ? showLogin : create)) {
                    CaperIcon(name: "plus", size: 20).foregroundStyle(CaperTheme.terracottaBright)
                        .frame(width: narrow ? 44 : 40, height: narrow ? 44 : 40)
                        .background(CaperTheme.surface)
                        .clipShape(RoundedRectangle(cornerRadius: 12))
                        .overlay(RoundedRectangle(cornerRadius: 12).stroke(CaperTheme.border, style: StrokeStyle(lineWidth: 1, dash: [4])))
                }
                // Stays enabled at the space limit: the dialog also joins by invite link.
                .buttonStyle(.plain).modifier(ControlHover())
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
    private let corner: CGFloat = 12
    private var fill: Color { selected ? Color(red: 57/255, green: 35/255, blue: 30/255) : CaperTheme.surface }
    private var edge: Color { selected ? Color(red: 128/255, green: 81/255, blue: 67/255) : CaperTheme.border }
    private var muted: Bool { space.demo != true && model.notificationsMuted(.space(space.id)) }
    private var unread: Bool { model.unreadSpaces.contains { $0.id == space.id } }
    private var state: String {
        let base: String = model.openingSpaceID == space.id ? "Opening" : selected ? "Selected" : ""
        return [base, unread ? "Unread activity" : "", muted ? "Muted" : ""].filter { !$0.isEmpty }.joined(separator: ", ")
    }

    var body: some View {
        Button { if !SwipeTapGuard.swiping { Task { await model.select(space: space) } } } label: {
            Text(space.demo == true ? "C" : String(space.name.prefix(1)).uppercased())
                .font(CaperTheme.font(13, weight: .black))
                .opacity(muted ? 0.5 : 1)
                .frame(width: narrow ? 44 : 40, height: narrow ? 44 : 40)
                .background(fill)
                .clipShape(RoundedRectangle(cornerRadius: corner))
                .overlay(RoundedRectangle(cornerRadius: corner).stroke(edge))
                .overlay(alignment: .bottomTrailing) { mutedBadge }
                .overlay(alignment: .topTrailing) {
                    if unread {
                        Circle().fill(CaperTheme.text).frame(width: 8, height: 8)
                            .overlay(Circle().stroke(CaperTheme.blackout, lineWidth: 2))
                            .offset(x: 2, y: -2).accessibilityHidden(true)
                    }
                }
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

    /// A muted space: a small bell-slash on the rail tile.
    @ViewBuilder private var mutedBadge: some View {
        if muted {
            MutedBell(size: 9)
                .frame(width: 16, height: 16)
                .background(Circle().fill(CaperTheme.blackout))
                .offset(x: 4, y: 4)
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
    @State private var searchFrame: CGRect = .null
    @State private var browsing = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// People often type the name as it is shown, with its `#` (as on web).
    private var channelQuery: String {
        channelSearch.trimmingCharacters(in: .whitespaces).drop(while: { $0 == "#" }).trimmingCharacters(in: .whitespaces)
    }
    #if os(macOS)
    @State private var directHeadingHovered = false
    @FocusState private var directActionFocused: Bool
    #endif
    /// Web's Browse channels search: trimmed, without leading "#"s.
    private var browseSearch: String {
        let trimmed = channelSearch.trimmingCharacters(in: .whitespacesAndNewlines)
        return String(trimmed.drop(while: { $0 == "#" })).trimmingCharacters(in: .whitespacesAndNewlines)
    }
    private var browseResults: [Channel] {
        let search = browseSearch
        return (model.detail?.channels ?? []).filter { search.isEmpty || $0.name.localizedCaseInsensitiveContains(search) }
    }
    /// The space menu: Browse, notifications and mute, then settings or leave.
    @ViewBuilder private var spaceMenuItems: some View {
        Button(browsing ? "Exit Browse channels" : "Browse channels") {
            browsing.toggle()
            channelSearch = ""
        }
        if let space = model.detail?.space, space.demo != true, model.account != nil {
            Divider()
            SpaceNotificationItems(model: model, spaceID: space.id)
            Divider()
        }
        if model.isOwner { Button("Space settings") { sheet = .manageSpace } }
        else if model.account != nil && model.detail?.space.demo != true { Button("Leave space…", role: .destructive) { sheet = .leaveSpace } }
    }
    var body: some View {
        VStack(spacing: 0) {
                    HStack(spacing: 6) {
                        Menu {
                            spaceMenuItems
                        } label: {
                            HStack {
                                Text(model.detail?.space.demo == true ? "Caper" : model.detail?.space.name ?? "Caper").font(CaperTheme.font(15, weight: .bold)).lineLimit(1)
                                Spacer(); if model.detail?.space.demo != true { CaperIcon(name: "chevron-down") }
                            }.contentShape(Rectangle())
                        }.menuStyle(.borderlessButton).menuIndicator(.hidden).disabled(model.detail?.space.demo == true)
                            .foregroundStyle(CaperTheme.text)
                            .modifier(ControlPointer())
                            .accessibilityLabel(model.detail?.space.demo == true ? "Caper" : model.detail?.space.name ?? "Caper")
                            .accessibilityIdentifier("selected-space-name")
                        if narrow { Button(action: SwipeTapGuard.unlessSwiping(close)) { CaperIcon(name: "x") }.buttonStyle(SidebarIconButton()).accessibilityLabel("Close navigation") }
                    }
                    .padding(.horizontal, 16).frame(height: 50)
                    .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
            NotificationErrorRow(model: model)

            ScrollView {
                if narrow && model.showingFeedback && model.isFeedbackTeam {
                    // Phones push the feedback inbox in place of the channel list.
                    FeedbackInboxList(model: model, pushed: true)
                } else if narrow && model.showingMessageRequests && model.account != nil {
                    // Phones push the requests list in place of the channel list.
                    MessageRequestsList(model: model, pushed: true)
                } else {
                VStack(spacing: 0) {
                VStack(alignment: .leading, spacing: 0) {
                    HStack {
                        Button { if !SwipeTapGuard.swiping { channelsExpanded.toggle(); CaperEffects.shared.toggle(channelsExpanded) } } label: {
                            HStack(spacing: 6) {
                                // One chevron that turns, as for message requests and voice rosters.
                                CaperIcon(name: "chevron-down")
                                    .rotationEffect(.degrees(channelsExpanded ? 0 : -90))
                                    .animation(reduceMotion ? nil : .easeOut(duration: 0.15), value: channelsExpanded)
                                Text("Channels")
                                Text("\(model.detail?.channels.filter(\.joined).count ?? 0)").font(CaperTheme.font(10, weight: .bold))
                            }.font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                                // The header's full 44pt height is the target, not just its text.
                                .frame(maxHeight: .infinity).contentShape(Rectangle())
                        }.buttonStyle(.plain).modifier(ControlHover())
                            .accessibilityValue(channelsExpanded ? "Expanded" : "Collapsed")
                        Spacer()
                        if model.isOwner {
                            let createHelp = model.canCreateChannel ? "Create channel" : "Channel limit reached (\(model.limits?.channelsPerSpace ?? 100))"
                            Button { if !SwipeTapGuard.swiping { sheet = .createChannel } } label: { CaperIcon(name: "plus") }
                                .buttonStyle(SidebarIconButton()).disabled(!model.canCreateChannel).help(createHelp).accessibilityLabel("Create channel")
                            // Web's owner-only Channel options menu.
                            Menu {
                                Button("Create channel") { sheet = .createChannel }.disabled(!model.canCreateChannel)
                                Button("\(channelsExpanded ? "Collapse" : "Expand") channels") { channelsExpanded.toggle() }
                            } label: { CaperIcon(name: "ellipsis") }
                                .menuStyle(.borderlessButton).menuIndicator(.hidden)
                                #if os(iOS)
                                .frame(width: 44, height: 44)
                                #else
                                .frame(width: 28, height: 28)
                                #endif
                                .contentShape(Rectangle()).modifier(ControlHover())
                                .help("Channel options").accessibilityLabel("Channel options")
                        }
                    }.frame(height: 44)

                    if channelsExpanded {
                        VStack(spacing: 3) {
                            ForEach(model.detail?.channels.filter(\.joined) ?? []) { channel in
                                ChannelSidebarItem(model: model, sheet: $sheet, channel: channel)
                            }
                        }
                    }

                    if browsing {
                    TextField("Search channels", text: $channelSearch).textFieldStyle(CaperTextFieldStyle()).padding(.vertical, 6)
                        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { searchFrame = $0 }
                    if browseResults.isEmpty {
                        Text(channelSearch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "This space has no channels yet." : "No channels match your search.")
                            .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted).padding(.vertical, 8)
                            .accessibilityIdentifier("browse-channels-empty")
                    }
                    ForEach(browseResults) { channel in
                        HStack(spacing: 4) {
                            // Includes unjoined channels, such as one where you were mentioned.
                            let unread = model.channelUnread(channel.id)
                            Button { if !SwipeTapGuard.swiping { Task { await model.select(channel: channel) } } } label: {
                                HStack {
                                    CaperIcon(name: channel.private ? "lock" : "hash", size: 16); Text(channel.name).lineLimit(1); Spacer()
                                    if unread { Circle().fill(CaperTheme.text).frame(width: 6, height: 6).accessibilityHidden(true) }
                                    Text(channel.joined ? "Joined" : "Read-only preview").font(CaperTheme.font(10))
                                }
                                // The whole row is the target, including the gap before Joined/Preview.
                                .padding(.vertical, 6)
                                #if os(iOS)
                                .frame(minHeight: 44)
                                #endif
                                .contentShape(Rectangle())
                            }.buttonStyle(.plain).modifier(ControlHover())
                                .accessibilityValue(unread ? "Unread activity" : "")
                            // Web's owner-only Manage button, also for channels you haven't joined.
                            if model.isOwner {
                                Button { if !SwipeTapGuard.swiping { sheet = .manageChannel(channel) } } label: { CaperIcon(name: "settings", size: 16) }
                                    .buttonStyle(SidebarIconButton()).help("Manage \(channel.name)")
                                    .accessibilityLabel("Manage \(channel.name)")
                            }
                        }
                    }
                    }
                    if let invitations = model.detail?.channelInvitations, !invitations.isEmpty {
                        Text("Private invitations").font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted).padding(.top, 12)
                        ForEach(invitations) { invitation in
                            Button { if !SwipeTapGuard.swiping { sheet = .channelInvitation(invitation) } } label: {
                                // Satoshi like the channel rows, with the whole row as the target.
                                Text("#\(invitation.channel.name) · from @\(invitation.inviter.username)").lineLimit(1)
                                    .font(CaperTheme.font(13, weight: .medium))
                                    .frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 6)
                                    #if os(iOS)
                                    .frame(minHeight: 44)
                                    #endif
                                    .contentShape(Rectangle())
                            }.buttonStyle(.plain).modifier(ControlHover())
                        }
                    }

                    if let error = model.error ?? model.directMessagesError {
                        Text(error).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error).padding(8)
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
            if model.account != nil {
                #if os(macOS)
                let directRowHeight: CGFloat = 28
                #else
                let directRowHeight: CGFloat = 44
                #endif
                VStack(spacing: 0) {
                    HStack {
                        Text("Direct messages").font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                        Spacer()
                        Button { if !SwipeTapGuard.swiping { sheet = .newDirectMessage } } label: { CaperIcon(name: "plus") }
                            .buttonStyle(SidebarIconButton()).accessibilityLabel("New direct message")
                            #if os(macOS)
                            .focused($directActionFocused)
                            .opacity(directHeadingHovered || directActionFocused ? 1 : 0)
                            .allowsHitTesting(directHeadingHovered || directActionFocused)
                            #endif
                    }.padding(.horizontal, 16).frame(height: directRowHeight)
                        #if os(macOS)
                        .contentShape(Rectangle())
                        .onHover { directHeadingHovered = $0 }
                        #endif
                        VStack(spacing: 2) {
                            if model.isFeedbackTeam {
                                FeedbackRow(model: model, narrow: narrow, rowHeight: directRowHeight)
                                if !narrow && model.showingFeedback {
                                    FeedbackInboxList(model: model, pushed: false)
                                }
                            }
                            if !model.messageRequests.isEmpty {
                                MessageRequestsRow(model: model, narrow: narrow, rowHeight: directRowHeight)
                                if !narrow && model.showingMessageRequests {
                                    MessageRequestsList(model: model, pushed: false)
                                }
                            }
                            if let account = model.account {
                                let selfConversation = model.directMessages.first { $0.peer.id == account.id }
                                Button { if !SwipeTapGuard.swiping { Task { await model.openSelfDirectMessage() } } } label: {
                                    HStack(spacing: 9) {
                                        Avatar(name: account.displayName ?? account.username ?? "You", size: 20, avatarID: account.avatarId).frame(width: 24, height: 20)
                                        Text(account.displayName ?? account.username ?? "You").lineLimit(1)
                                        Text("you").font(CaperTheme.font(10, weight: .bold)).foregroundStyle(CaperTheme.muted)
                                            .fixedSize(horizontal: true, vertical: false)
                                        Spacer()
                                        if selfConversation?.unread == true { Circle().fill(CaperTheme.terracottaBright).frame(width: 8, height: 8).accessibilityLabel("Unread") }
                                    }.font(CaperTheme.font(13, weight: .medium))
                                        .foregroundStyle(selfConversation.map { model.selectedDirectMessageID == $0.id } == true ? CaperTheme.text : CaperTheme.muted)
                                        .padding(.horizontal, 9).frame(height: directRowHeight)
                                        .background(selfConversation.map { model.selectedDirectMessageID == $0.id } == true ? CaperTheme.terracotta.opacity(0.16) : Color.clear)
                                        .clipShape(RoundedRectangle(cornerRadius: 6))
                                        .contentShape(Rectangle())
                                }.buttonStyle(.plain).disabled(model.busy)
                                    .modifier(NavigationPrefetchModifier {
                                        if let selfConversation { model.prefetch(directMessage: selfConversation) }
                                    })
                                    .accessibilityIdentifier("dm-self")
                            }
                            ForEach(model.visibleDirectMessages.filter { $0.peer.id != model.account?.id }) { conversation in
                                DirectMessageSidebarRow(model: model, conversation: conversation, rowHeight: directRowHeight)
                            }
                        }.padding(.horizontal, 16).padding(.top, 2)
                    Button {
                        guard !SwipeTapGuard.swiping else { return }
                        if model.isOwner, model.detail?.space.demo == false { sheet = .manageSpace }
                        else { sheet = .newDirectMessage }
                    } label: {
                        HStack(spacing: 9) {
                            CaperIcon(name: "plus", size: 17).frame(width: 24)
                            Text(model.isOwner && model.detail?.space.demo == false ? "Invite people" : "New message")
                            Spacer()
                        }
                        .font(CaperTheme.font(13, weight: .medium)).foregroundStyle(CaperTheme.muted)
                        .padding(.horizontal, 9).frame(minHeight: directRowHeight)
                        .contentShape(Rectangle())
                    }.buttonStyle(.plain).modifier(ControlHover()).padding(.horizontal, 16)
                        .accessibilityIdentifier(model.isOwner && model.detail?.space.demo == false ? "invite-people" : "new-message")
                }.padding(.bottom, 8)
                    .overlay(alignment: .top) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
            }
                }
                }
            }
            .modifier(FeedbackPullToRefresh(enabled: narrow && model.showingFeedback && model.isFeedbackTeam) { await model.refreshFeedback() })
            .accessibilityIdentifier("channel-browser")
            if !narrow, model.voice.phase != .idle, model.voice.phase != .failed {
                Divider().overlay(CaperTheme.border)
            }
            if !narrow { AccountBar(model: model, sheet: $sheet) }
        }
        .modifier(BrowseSwipe(open: true, enabled: narrow && sheet == nil && (model.selectedChannel != nil || model.selectedDirectMessageID != nil || model.visit != nil), excludedStart: browsing ? searchFrame : .null, navigate: close))
        // Browse belongs to the space it was opened in.
        .onChange(of: model.selectedSpaceID) { _, _ in browsing = false; channelSearch = "" }
        .background(CaperTheme.sidebar)
        .overlay(alignment: .trailing) { if !narrow { Rectangle().fill(CaperTheme.border).frame(width: 1) } }
    }
}

/// One DM in the sidebar, with its options menu: a context menu (press and
/// hold on iPhone), plus a "…" on hover on macOS. Personal notes have none.
private struct DirectMessageSidebarRow: View {
    @Bindable var model: AppModel
    let conversation: DirectMessageConversation
    let rowHeight: CGFloat
    /// The whole row, options included: its hover wash spans them, like web.
    @State private var hovered = false
    #if os(macOS)
    @FocusState private var optionsFocused: Bool
    #endif

    private var selected: Bool { model.selectedDirectMessageID == conversation.id }
    private var muted: Bool { model.notificationsMuted(.direct(conversation.id)) }
    /// A muted DM shows no unread dot.
    private var showsUnread: Bool { conversation.unread && !muted }
    private var rowState: String {
        let base: String = selected ? "Selected" : showsUnread ? "Unread" : ""
        guard muted else { return base }
        return base.isEmpty ? "Muted" : base + ", Muted"
    }

    var body: some View {
        HStack(spacing: 2) {
            rowButton
            #if os(macOS)
            optionsMenu
            #endif
        }
        // Selection and hover span the row, trailing options included.
        .background { RoundedRectangle(cornerRadius: 6).fill(rowFill).allowsHitTesting(false) }
        .onHover { hovered = $0 }
        .contextMenu { DirectMessageNotificationItems(model: model, conversationID: conversation.id) }
    }

    private var rowFill: Color {
        selected ? CaperTheme.terracotta.opacity(0.16) : hovered ? Color.white.opacity(0.06) : .clear
    }

    private var rowButton: some View {
        Button { if !SwipeTapGuard.swiping { Task { await model.select(directMessage: conversation) } } } label: { rowLabel }
            .buttonStyle(.plain).accessibilityIdentifier("dm-\(conversation.id)")
            .modifier(NavigationPrefetchModifier(wash: false) { model.prefetch(directMessage: conversation) })
            .accessibilityValue(rowState)
    }

    private var rowLabel: some View {
        HStack(spacing: 9) {
            Avatar(name: conversation.peer.displayName, size: 20, avatarID: conversation.peer.avatarId).frame(width: 24, height: 20)
            Text(conversation.peer.displayName).lineLimit(1)
            if muted { MutedBell() }
            Spacer()
            if showsUnread { Circle().fill(CaperTheme.terracottaBright).frame(width: 8, height: 8).accessibilityLabel("Unread") }
        }
        .font(CaperTheme.font(13, weight: .medium))
        .foregroundStyle(selected ? CaperTheme.text : CaperTheme.muted)
        .opacity(muted ? 0.55 : 1)
        .padding(.horizontal, 9).frame(height: rowHeight)
        .contentShape(Rectangle())
    }

    #if os(macOS)
    private var optionsMenu: some View {
        Menu { DirectMessageNotificationItems(model: model, conversationID: conversation.id) } label: { CaperIcon(name: "ellipsis") }
            .menuStyle(.borderlessButton).menuIndicator(.hidden)
            .foregroundStyle(CaperTheme.muted)
            .frame(width: 28, height: 28)
            .focused($optionsFocused)
            .opacity(hovered || optionsFocused ? 1 : 0)
            .allowsHitTesting(hovered || optionsFocused)
            // Hover feedback like the channel rows' options.
            .contentShape(Rectangle()).modifier(ControlHover(isFocused: optionsFocused))
            .help("Conversation options for \(conversation.peer.displayName)")
            .accessibilityLabel("Conversation options for \(conversation.peer.displayName)")
            .accessibilityIdentifier("dm-options-\(conversation.id)")
    }
    #endif
}

private struct ChannelSidebarItem: View {
    @Bindable var model: AppModel
    @Binding var sheet: WorkspaceSheet?
    let channel: Channel
    @State private var confirmLeave = false
    /// The channel's line (name, quick join and options): its hover wash spans
    /// them, like web's `.channel-line`, but not the voice roster below.
    @State private var lineHovered = false
    #if os(macOS)
    @State private var hovered = false
    @FocusState private var optionsFocused: Bool
    @FocusState private var joinFocused: Bool
    #endif

    private var sessionStartedAt: Double? {
        let shared = model.voicePresence.sessionStartedAt(for: channel.id)
        if model.voice.isActive(channelID: channel.id) {
            return model.voice.phase == .joining ? shared ?? model.voice.sessionStartedAt : model.voice.sessionStartedAt
        }
        return shared
            ?? (model.pendingVoiceChannelID == channel.id ? Double(model.pendingVoiceStartedAt) : nil)
    }

    private var muted: Bool { model.notificationsMuted(.channel(spaceID: channel.spaceId, channelID: channel.id)) }
    /// Muted channels are dimmed, with a bell-slash.
    private var nameOpacity: Double { muted ? 0.55 : 1 }
    private var rowState: String {
        let base: String = model.openingChannelID == channel.id ? "Opening" : model.selectedChannelID == channel.id ? "Selected" : ""
        return [base, unread ? "Unread activity" : "", muted ? "Muted" : ""].filter { !$0.isEmpty }.joined(separator: ", ")
    }
    private var unread: Bool { model.channelUnread(channel.id) }

    @ViewBuilder private var optionsMenuItems: some View {
        if model.isOwner {
            Button("Channel settings") { sheet = .manageChannel(channel) }
            Divider()
        }
        if model.account != nil { ChannelNotificationItems(model: model, channel: channel) }
        if channel.joined {
            Divider()
            Button("Leave channel") { confirmLeave = true }
        }
    }

    private var lineFill: Color {
        model.selectedChannelID == channel.id ? CaperTheme.terracotta.opacity(0.16) : lineHovered ? Color.white.opacity(0.06) : .clear
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 2) {
                Button { if !SwipeTapGuard.swiping { Task { await model.select(channel: channel) } } } label: {
                    HStack(spacing: 6) {
                        CaperIcon(name: channel.private ? "lock" : "hash", size: 18)
                            .foregroundStyle(model.selectedChannelID == channel.id ? CaperTheme.terracottaBright : CaperTheme.muted)
                            .opacity(nameOpacity)
                        Text(channel.name).lineLimit(1).opacity(nameOpacity)
                        Spacer(minLength: 0)
                        // Web and Android order: the unread dot, then the bell.
                        if unread { Circle().fill(CaperTheme.text).frame(width: 6, height: 6).accessibilityHidden(true) }
                        if muted { MutedBell() }
                        if let sessionStartedAt {
                            TimelineView(.periodic(from: .now, by: 1)) { context in
                                Text(VoiceSessionDuration.format(startedAtMilliseconds: sessionStartedAt, now: context.date))
                                    .font(CaperTheme.font(11, weight: .medium).monospacedDigit())
                                    .foregroundStyle(CaperTheme.voiceSessionGreen)
                                    .fixedSize(horizontal: true, vertical: false)
                                    .accessibilityLabel("Voice session duration")
                                    .accessibilityValue(VoiceSessionDuration.format(startedAtMilliseconds: sessionStartedAt, now: context.date))
                            }
                        }
                    }
                    .font(CaperTheme.font(13, weight: .medium))
                    .foregroundStyle(model.selectedChannelID == channel.id ? CaperTheme.text : CaperTheme.muted)
                    .padding(.horizontal, 9)
                    #if os(iOS)
                    .frame(height: 44)
                    #else
                    .frame(height: 32)
                    #endif
                    .contentShape(Rectangle())
                }.buttonStyle(.plain)
                    .modifier(NavigationPrefetchModifier(wash: false) {
                        if let space = model.detail?.space { model.prefetch(space: space, channelID: channel.id) }
                    })
                    .accessibilityIdentifier("channel-\(channel.id)")
                    .accessibilityValue(rowState)
                #if os(macOS)
                quickJoin
                #endif
                if model.detail?.space.demo != true {
                    Menu {
                        optionsMenuItems
                    } label: { CaperIcon(name: "ellipsis") }
                        .menuStyle(.borderlessButton).menuIndicator(.hidden)
                        .foregroundStyle(CaperTheme.muted)
                        #if os(iOS)
                        .frame(width: 44, height: 44)
                        #else
                        .frame(width: 32, height: 32)
                        .focused($optionsFocused)
                        .opacity(hovered || optionsFocused ? 1 : 0)
                        .allowsHitTesting(hovered || optionsFocused)
                        #endif
                        .contentShape(Rectangle()).modifier(ControlHover())
                        .help("Channel options for \(channel.name)")
                        .accessibilityLabel("Channel options for \(channel.name)")
                        .accessibilityIdentifier("channel-options-\(channel.id)")
                }
            }
            // Selection and hover span the line, trailing controls included.
            .background { RoundedRectangle(cornerRadius: 6).fill(lineFill).allowsHitTesting(false) }
            .onHover { lineHovered = $0 }
            ChannelVoiceSlot(model: model, channel: channel)
        }
        #if os(macOS)
        .onHover { hovered = $0 }
        #endif
        .task(id: channel.id) { await model.refreshVoiceAvailability(channel: channel) }
        .sheet(isPresented: $confirmLeave) {
            ConfirmationSheet(title: "Leave #\(channel.name)?", detail: channel.private && !model.isOwner ? "You’ll lose access and need another invitation to return. You’ll disconnect from this channel’s voice call." : "It will leave your sidebar. You can preview and rejoin from Browse channels. You’ll disconnect from this channel’s voice call.", action: "Leave channel", pendingLabel: "Leaving…", close: { confirmLeave = false }) { try await model.leaveChannel(channel) }
                .modifier(ConfirmationPresentation())
        }
    }

    #if os(macOS)
    /// Web's channel-quick-join: a pointer shortcut revealed by hover or keyboard
    /// focus like the row menu. Its slot is always kept, so nothing shifts; your
    /// own call's channel has none. Touch joins from the conversation header.
    @ViewBuilder private var quickJoin: some View {
        let action = ChannelVoiceAction(model: model, channel: channel)
        if action.here {
            Color.clear.frame(width: 28, height: 28).accessibilityHidden(true)
        } else {
            Button { Task { await model.joinVoice(channel: channel) } } label: {
                CaperIcon(name: "headphones", size: 16).frame(width: 28, height: 28).contentShape(Rectangle())
            }.buttonStyle(.plain)
                .foregroundStyle(CaperTheme.muted)
                .disabled(!action.enabled)
                .focused($joinFocused)
                .opacity((hovered || joinFocused) ? (action.enabled ? 1 : 0.45) : 0)
                .allowsHitTesting(hovered || joinFocused)
                .modifier(ControlHover(isFocused: joinFocused))
                .help(action.help)
                .accessibilityLabel(action.description(header: false))
                .accessibilityIdentifier("join-voice-\(channel.id)")
                .modifier(PrepareVoiceOnApproach { model.prepareVoiceJoin(channel: channel) })
                .onChange(of: joinFocused) { _, focused in if focused { model.prepareVoiceJoin(channel: channel) } }
        }
    }
    #endif
}

enum VoiceSessionDuration {
    static func format(startedAtMilliseconds: Double, now: Date) -> String {
        let seconds = max(0, Int(now.timeIntervalSince1970 - startedAtMilliseconds / 1_000))
        let hours = seconds / 3_600
        let minutes = (seconds % 3_600) / 60
        let remainder = seconds % 60
        if hours > 0 { return String(format: "%d:%02d:%02d", hours, minutes, remainder) }
        return String(format: "%02d:%02d", minutes, remainder)
    }
}

/// Prefetches a navigation target on pointer approach or keyboard focus
/// (macOS), and gives the control its hover and focus feedback. A row that
/// draws its own hover wash across its trailing controls passes `wash: false`.
private struct NavigationPrefetchModifier: ViewModifier {
    var wash = true
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
            .modifier(ControlHover(isFocused: focused, wash: wash))
        #else
        content.modifier(ControlHover(wash: wash))
        #endif
    }
}

private struct SidebarIconButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.system(size: 14, weight: .semibold)).foregroundStyle(CaperTheme.muted)
            #if os(iOS)
            .frame(width: 44, height: 44)
            #else
            .frame(width: 28, height: 28)
            #endif
            // Press feedback on touch as well as with a pointer.
            .background(configuration.isPressed ? CaperTheme.border : .clear)
            .clipShape(RoundedRectangle(cornerRadius: 5))
            .modifier(ControlHover())
    }
}

/// Who is in voice in a channel: your own call's participants while you are in
/// it, else the read-only presence roster.
@MainActor private func voiceOccupants(_ model: AppModel, channelID: String) -> [VoiceSpectator] {
    // `voiceRoster` also covers a visited space channel, outside the open space.
    guard model.voice.isActive(channelID: channelID) else { return model.voiceRoster(for: channelID) }
    return model.voice.participants.map {
        VoiceSpectator(id: $0.id, avatarId: $0.avatarId, name: $0.name, muted: $0.muted, deafened: $0.deafened)
    }
}

/// Web's voiceAction: join, switch or leave voice for one channel. The
/// conversation header labels it on every device; macOS rows add a shortcut.
@MainActor private struct ChannelVoiceAction {
    let model: AppModel
    let channel: Channel

    /// Your call is in this channel, from joining until it has left.
    var active: Bool { model.voice.isActive(channelID: channel.id) }
    /// Your call is here or on its way: rows offer no shortcut.
    var here: Bool { active || model.pendingVoiceChannelID == channel.id }
    /// Connected (or reconnecting) here: the header offers Leave instead of Join.
    var leaves: Bool { active && (model.voice.phase == .connected || model.voice.phase == .reconnecting) }
    private var pending: Bool {
        model.pendingVoiceChannelID != nil || model.voice.phase == .joining || model.voice.phase == .reconnecting || model.voice.phase == .leaving
    }
    var enabled: Bool { !pending && model.voiceAvailable(in: channel) == true }
    var title: String {
        if model.pendingVoiceChannelID == channel.id { return "Joining…" }
        if active { return model.voice.phase == .leaving ? "Leaving…" : "Joining…" }
        return model.voice.phase == .idle || model.voice.phase == .failed ? "Join voice" : "Switch here"
    }
    /// The header's Join is already beside the channel's name; a row's names its channel.
    func description(header: Bool) -> String {
        switch title {
        case "Joining…": return "Joining voice in #\(channel.name)"
        case "Leaving…": return "Leaving voice in #\(channel.name)"
        case "Switch here": return "Switch voice to #\(channel.name)"
        default: return header ? "Join voice" : "Join voice in #\(channel.name)"
        }
    }
    var help: String {
        let named = description(header: false)
        return active ? named : voiceAvailabilityHelp(model.voiceAvailable(in: channel)) ?? named
    }
}

/// An occupied channel's voice line: faces and "N in voice", expanding to who is
/// there. Empty channels add nothing beneath their name.
private struct ChannelVoiceSlot: View {
    @Bindable var model: AppModel
    let channel: Channel
    @State private var collapsed = true

    var body: some View {
        let occupants = voiceOccupants(model, channelID: channel.id)
        let active = model.voice.isActive(channelID: channel.id)
        if !occupants.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Button { if !SwipeTapGuard.swiping { collapsed.toggle() } } label: {
                        ViewThatFits(in: .horizontal) {
                            HStack(spacing: 6) {
                                HStack(spacing: -6) {
                                    ForEach(occupants.prefix(2)) { person in
                                        Avatar(name: person.name, size: 20, avatarID: person.avatarId,
                                               speaking: active && model.voice.speakingParticipants.contains(person.id))
                                    }
                                }
                                Text("\(occupants.count) in voice")
                                CaperIcon(name: "chevron-down", size: 11).rotationEffect(.degrees(collapsed ? -90 : 0))
                            }.fixedSize(horizontal: true, vertical: false)
                            HStack(spacing: 6) {
                                Text("\(occupants.count) in voice")
                                CaperIcon(name: "chevron-down", size: 11).rotationEffect(.degrees(collapsed ? -90 : 0))
                            }.fixedSize(horizontal: true, vertical: false)
                        }
                        .font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.muted)
                        #if os(iOS)
                        .frame(minWidth: 44, minHeight: 44, alignment: .leading)
                        #else
                        .frame(minWidth: 42, minHeight: 28, alignment: .leading)
                        #endif
                        .contentShape(Rectangle())
                    }.buttonStyle(.plain).modifier(ControlHover())
                        .accessibilityLabel("\(occupants.count) in voice in \(channel.name). \(collapsed ? "Show" : "Hide") who is in voice")
                        .accessibilityIdentifier("voice-stack-\(channel.id)")
                        .accessibilityValue(collapsed ? "Collapsed" : "Expanded")
                    Spacer(minLength: 0)
                }
                .padding(.trailing, 4)
                if !collapsed {
                    if active {
                        VoiceRoster(model: model)
                    } else {
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
                }
            }.padding(.bottom, 2)
        }
    }
}

/// Whether you are in a call, from joining until it has left.
@MainActor private func inVoiceCall(_ voice: VoiceClient) -> Bool {
    voice.phase != .idle && voice.phase != .failed
}

/// Web's headerVoice: who is in voice in the open channel, then Join voice,
/// Switch here or Leave. Phones show only icon buttons (with the call's microphone
/// toggle, since the account bar stays in Browse) and no faces; wider headers
/// drop to one face only when the title needs the room.
private struct HeaderVoice: View {
    @Bindable var model: AppModel
    let channel: Channel
    let narrow: Bool

    /// "Maya is in voice", "Maya and Alex are in voice", "A, B, C and 2 more are in voice".
    static func summary(_ names: [String]) -> String {
        if names.count > 3 { return "\(names.prefix(3).joined(separator: ", ")) and \(names.count - 3) more are in voice" }
        if names.count > 1 { return "\(names.dropLast().joined(separator: ", ")) and \(names[names.count - 1]) are in voice" }
        return "\(names.first ?? "") is in voice"
    }

    var body: some View {
        let people = voiceOccupants(model, channelID: channel.id)
        let action = ChannelVoiceAction(model: model, channel: channel)
        Group {
            if narrow {
                // Phones: one bordered group of icons beside the name, as in Slack. The
                // labels stay as accessible names. No faces: a lone face was unclear, and
                // the title's subtitle already counts who is in voice.
                HStack(spacing: 2) {
                    if inVoiceCall(model.voice) {
                        HeaderMuteButton(voice: model.voice)
                        HeaderDeafenButton(voice: model.voice)
                    }
                    if action.leaves { HeaderLeaveButton(model: model, compact: true) }
                    else { join(action, compact: true, occupied: !people.isEmpty) }
                }.modifier(CompactVoiceGroup())
            } else {
                HStack(spacing: 10) {
                    if !people.isEmpty { occupants(people, speaking: action.active) }
                    if action.leaves { HeaderLeaveButton(model: model, compact: false) }
                    else { join(action, compact: false, occupied: !people.isEmpty) }
                }
            }
        }
        .task(id: channel.id) { await model.refreshVoiceAvailability(channel: channel) }
    }

    private func occupants(_ people: [VoiceSpectator], speaking active: Bool) -> some View {
        let summary = Self.summary(people.map(\.name))
        return ViewThatFits(in: .horizontal) {
            faces(people, limit: 3, count: true, speaking: active)
            faces(people, limit: 1, count: false, speaking: active)
        }
        .help(summary)
        .accessibilityElement(children: .ignore)
        .accessibilityAddTraits(.isImage)
        .accessibilityLabel(summary)
        .accessibilityIdentifier("header-voice-people")
    }

    /// Join voice, Switch here or Joining…: labelled on wider layouts, an icon on phones.
    /// While people are talking here and you are not in this channel's call, it glows.
    @ViewBuilder private func join(_ action: ChannelVoiceAction, compact: Bool, occupied: Bool) -> some View {
        let glows = occupied && action.enabled && !action.active
        let button = Button { if !SwipeTapGuard.swiping { Task { await model.joinVoice(channel: channel) } } } label: {
            if compact {
                CaperIcon(name: "headphones", size: 20)
            } else {
                HStack(spacing: 6) {
                    CaperIcon(name: "headphones", size: 14)
                    Text(action.title).lineLimit(1)
                }
            }
        }
        Group {
            if compact {
                button.buttonStyle(HeaderVoiceIconButton(tint: CaperTheme.terracottaBright,
                                                         fill: glows ? CaperTheme.terracotta.opacity(0.16) : .clear))
            } else {
                button.buttonStyle(HeaderVoiceButton(primary: true)).fixedSize()
            }
        }
        .modifier(VoiceJoinGlow(active: glows))
        .disabled(!action.enabled)
        .help(action.help)
        .accessibilityLabel(action.description(header: true))
        .accessibilityIdentifier("join-voice-button")
        .modifier(PrepareVoiceOnApproach { model.prepareVoiceJoin(channel: channel) })
    }

    private func faces(_ people: [VoiceSpectator], limit: Int, count: Bool, speaking active: Bool) -> some View {
        HStack(spacing: 6) {
            HStack(spacing: -6) {
                ForEach(people.prefix(limit)) { person in
                    Avatar(name: person.name, size: 24, avatarID: person.avatarId,
                           speaking: active && model.voice.speakingParticipants.contains(person.id))
                }
            }
            if count {
                Text("\(people.count)").font(CaperTheme.font(12, weight: .bold).monospacedDigit())
                    .foregroundStyle(CaperTheme.muted)
            }
        }.fixedSize()
    }
}

/// Phones group the header's voice controls in one bordered container.
private struct CompactVoiceGroup: ViewModifier {
    func body(content: Content) -> some View {
        content.padding(.horizontal, 2)
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border, lineWidth: 1))
            .fixedSize()
    }
}

/// Web's #ff7182: a muted microphone, deafened audio and the header's disconnect.
private let headerVoiceAlert = Color(red: 1, green: 113/255, blue: 130/255)

/// Web's voice-join-glow: a soft terracotta glow pulsing on a 2.4s cycle while
/// people are in voice here and you are not; Reduce Motion keeps it still.
private struct VoiceJoinGlow: ViewModifier {
    let active: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var lit = false
    func body(content: Content) -> some View {
        let opacity: Double = !active ? 0 : reduceMotion ? 0.35 : lit ? 0.45 : 0
        content
            .shadow(color: CaperTheme.terracottaBright.opacity(opacity), radius: reduceMotion ? 5 : 7)
            .task(id: active && !reduceMotion) {
                lit = false
                guard active && !reduceMotion else { return }
                // 1.2s each way: one 2.4s breath, as on web.
                withAnimation(.easeInOut(duration: 1.2).repeatForever(autoreverses: true)) { lit = true }
            }
    }
}

/// An icon-only 44-point control in a phone header's voice group.
private struct HeaderVoiceIconButton: ButtonStyle {
    var tint: Color = CaperTheme.muted
    var fill: Color = .clear
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.foregroundStyle(tint)
            .frame(width: 44, height: 44)
            .background(configuration.isPressed ? CaperTheme.raised : fill)
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .contentShape(Rectangle())
            .opacity(enabled ? 1 : 0.45).modifier(ControlHover())
    }
}

/// The call's microphone toggle in a phone's conversation header: the same
/// action as the account bar's, which stays in Browse.
private struct HeaderMuteButton: View {
    @Bindable var voice: VoiceClient
    var body: some View {
        Button {
            guard !SwipeTapGuard.swiping else { return }
            CaperEffects.shared.toggle(voice.muted); Task { await voice.setMuted(!voice.muted) }
        } label: {
            CaperIcon(name: voice.muted ? "mic-off" : "mic", size: 20)
        }
        // Web's phone header tints a muted microphone red (#ff7182).
        .buttonStyle(HeaderVoiceIconButton(tint: voice.muted ? headerVoiceAlert : CaperTheme.muted))
        .help(voice.muted ? "Unmute" : "Mute")
        .accessibilityLabel(voice.muted ? "Unmute microphone" : "Mute microphone")
        .accessibilityValue(voice.muted ? "Muted" : "On")
        .accessibilityIdentifier("header-mute")
    }
}

/// The call's deafen toggle beside the header's microphone on phones: the same
/// action as the account bar's.
private struct HeaderDeafenButton: View {
    @Bindable var voice: VoiceClient
    var body: some View {
        Button {
            guard !SwipeTapGuard.swiping else { return }
            CaperEffects.shared.toggle(voice.deafened); Task { await voice.setDeafened(!voice.deafened) }
        } label: {
            CaperIcon(name: voice.deafened ? "volume-x" : "headphones", size: 20)
        }
        .buttonStyle(HeaderVoiceIconButton(tint: voice.deafened ? headerVoiceAlert : CaperTheme.muted))
        .help(voice.deafened ? "Undeafen" : "Deafen")
        .accessibilityLabel(voice.deafened ? "Undeafen audio" : "Deafen audio")
        .accessibilityValue(voice.deafened ? "Deafened" : "On")
        .accessibilityIdentifier("header-deafen")
    }
}

/// Leave (or, before connecting, Cancel) the call from the conversation header:
/// labelled on wider layouts, an icon on phones.
private struct HeaderLeaveButton: View {
    let model: AppModel
    let compact: Bool
    var body: some View {
        let connected = model.voice.phase == .connected
        let button = Button {
            guard !SwipeTapGuard.swiping else { return }
            if connected { CaperEffects.shared.play(.disconnect) }
            model.leaveVoice()
        } label: {
            if compact {
                CaperIcon(name: "phone-off", size: 20)
            } else {
                HStack(spacing: 6) {
                    CaperIcon(name: "phone-off", size: 14)
                    Text(connected ? "Leave" : "Cancel").lineLimit(1)
                }
            }
        }
        Group {
            // Disconnect is a red icon and label everywhere in the header, with no fill or border, as on web.
            if compact { button.buttonStyle(HeaderVoiceIconButton(tint: headerVoiceAlert)) }
            else { button.buttonStyle(HeaderLeaveButtonStyle()).fixedSize() }
        }
        .help(connected ? "Disconnect" : "Cancel joining voice")
        .accessibilityLabel(connected ? "Leave voice" : "Cancel joining voice")
        .accessibilityIdentifier("header-leave-voice")
    }
}

/// The wider header's labelled disconnect: #ff7182 text and icon only; pressing
/// adds the neutral raised wash other header controls use.
private struct HeaderLeaveButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(CaperTheme.font(12, weight: .bold))
            .foregroundStyle(headerVoiceAlert)
            .padding(.horizontal, 10)
            #if os(iOS)
            .frame(minHeight: 44)
            #else
            .frame(minHeight: 30)
            #endif
            .background(configuration.isPressed ? CaperTheme.raised : Color.clear)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .contentShape(Rectangle())
            .modifier(ControlHover())
    }
}

/// The header's voice action: terracotta-tinted Join or Switch here.
private struct HeaderVoiceButton: ButtonStyle {
    let primary: Bool
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(CaperTheme.font(12, weight: .bold))
            .foregroundStyle(primary ? CaperTheme.terracottaBright : CaperTheme.muted)
            .padding(.horizontal, 10)
            #if os(iOS)
            .frame(minHeight: 44)
            #else
            .frame(minHeight: 30)
            #endif
            .background(primary ? CaperTheme.terracotta.opacity(configuration.isPressed ? 0.25 : 0.14)
                        : configuration.isPressed ? CaperTheme.composer : CaperTheme.raised)
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(primary ? CaperTheme.terracotta.opacity(0.7) : CaperTheme.border))
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .contentShape(Rectangle())
            .opacity(enabled ? 1 : 0.45).modifier(ControlHover())
    }
}

private struct MemberPresenceView: View {
    @Bindable var model: AppModel
    @Bindable var presence: PresenceModel
    var close: (() -> Void)?
    @State private var profile: MentionCardTarget?
    @State private var pointer = MentionPointer()
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
                    Button(action: SwipeTapGuard.unlessSwiping(close)) { CaperIcon(name: "x") }
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
                    Button {
                        // Swiping the phone panel closed lifts on the row it started on.
                        guard !SwipeTapGuard.swiping else { return }
                        profile = MentionCardTarget(pill: MentionPill(id: member.id, username: member.username), pointer: pointer.location)
                    } label: {
                        HStack(spacing: 10) {
                            ZStack(alignment: .bottomTrailing) {
                                Avatar(name: member.displayName, size: 30, avatarID: member.avatarId)
                                Circle().fill(statusColor(presence.status(for: member))).frame(width: 10, height: 10)
                                    .overlay(Circle().stroke(CaperTheme.sidebar, lineWidth: 2))
                            }
                            Text(member.displayName).font(CaperTheme.font(12, weight: .medium)).lineLimit(1)
                            Spacer(minLength: 0)
                        }.frame(maxWidth: .infinity, minHeight: 38, alignment: .leading)
                    }.buttonStyle(ProfileButtonStyle())
                        .accessibilityLabel("Open profile for \(member.displayName)")
                        #if os(macOS)
                        .onContinuousHover(coordinateSpace: CoordinateSpace.named(MentionCard.timelineSpace)) { phase in
                            switch phase {
                            case .active(let point): pointer.location = point
                            case .ended: pointer.location = nil
                            }
                        }
                        #endif
                }
                if presence.pageCount > 1 {
                    HStack {
                        Button("Previous") { if !SwipeTapGuard.swiping { Task { await presence.showPage(presence.page - 1) } } }.disabled(presence.page == 0)
                        Spacer(); Text("\(presence.page + 1) / \(presence.pageCount)")
                        Spacer(); Button("Next") { if !SwipeTapGuard.swiping { Task { await presence.showPage(presence.page + 1) } } }.disabled(presence.page + 1 >= presence.pageCount)
                    }.font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
                }
            }
            }.padding(.horizontal, 12)
            }
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(CaperTheme.sidebar)
            .overlay(alignment: .leading) { Rectangle().fill(CaperTheme.border).frame(width: 1) }
            .coordinateSpace(.named(MentionCard.timelineSpace))
            .modifier(MentionCardHost(target: $profile, model: model, viewerID: model.account?.id))
            .onChange(of: model.detail?.space.id) { _, _ in profile = nil }
    }
    private func statusColor(_ status: PresenceStatus) -> Color {
        switch status { case .online: CaperTheme.green; case .idle: Color(red: 0.72, green: 0.60, blue: 0.35); case .offline, .unknown: CaperTheme.border }
    }
}

/// The member list on phones, over the conversation below its header. It
/// slides in and out with Browse's curve and follows a finger the same way.
private struct MembersOverlay: View {
    @Bindable var model: AppModel
    let shows: Bool
    let width: CGFloat
    let close: () -> Void
    @Environment(MembersDrawer.self) private var drawer: MembersDrawer?
    /// Bumped once a closing slide ends, to drop the list it kept in view.
    @State private var settled = 0

    var body: some View {
        let drag = drawer?.drag
        let settle = drawer?.settle
        let shown = drag ?? (shows ? 1 : 0)
        // Still sliding: the list stays until it is out of view (`settled`).
        let moving = settled >= 0 && settle.map { !$0.finished() } == true
        // Past the screen's edge, so the panel's shadow leaves with it.
        let travel = width + 8 + 48
        let rest: CGFloat = shows ? 1 : 0
        ZStack(alignment: .trailing) {
            Color.black.opacity(0.25)
                .contentShape(Rectangle())
                .onTapGesture { if !SwipeTapGuard.swiping { close() } }
                // Each layer swipes closed within its own bounds, before its
                // margins: closed, the overlay must leave the conversation's
                // header and timeline gestures alone.
                .modifier(MembersSwipe(open: true, enabled: shows, navigate: close))
                .padding(.top, 50)
                .modifier(DrawerPose<MembersDrawer>(rest: rest, fallback: .tap) { (.zero, Double($0)) })
                .allowsHitTesting(shows)
                .accessibilityHidden(true)
            Group {
                // Only rendered while some of it is in view: hidden or transparent,
                // the closed list's "Members" header stayed in the iOS accessibility
                // tree for VoiceOver and the UI tests. Closing keeps the list until
                // it has slid away, rather than emptying the panel as it leaves.
                if shown > 0 || moving { MemberPresenceView(model: model, close: close) } else { Color.clear }
            }
                .frame(width: width)
                .clipShape(RoundedRectangle(cornerRadius: 16))
                // Outlined on every side, and lifted by the same shadow the conversation casts over Browse.
                .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(CaperTheme.border))
                .background {
                    RoundedRectangle(cornerRadius: 16).fill(CaperTheme.sidebar)
                        .shadow(color: .black.opacity(0.45), radius: 16, x: -12)
                }
                .modifier(MembersSwipe(open: true, enabled: shows, navigate: close))
                .padding(.top, 58).padding(.trailing, 8).padding(.bottom, 8)
                .modifier(DrawerPose<MembersDrawer>(rest: rest, fallback: .tap) { shown in
                    (CGSize(width: (1 - shown) * travel, height: 0), shown == 0 && !moving ? 0 : 1)
                })
                .allowsHitTesting(shows)
                .accessibilityElement(children: .contain)
                .accessibilityHidden(!shows)
        }
        .onChange(of: travel, initial: true) { _, travel in drawer?.travel = travel }
        .onChange(of: shown == 0) { _, closed in
            // Buttons and swipes record their slide first; `DrawerPose` records a model change's.
            guard closed, let settle = drawer?.settle, !settle.finished() else { return }
            let remaining = settle.timing.duration - Date().timeIntervalSince(settle.start)
            DispatchQueue.main.asyncAfter(deadline: .now() + remaining + 0.02) { settled += 1 }
        }
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
    let model: AppModel
    @State private var audioParticipantID: String? = nil
    init(model: AppModel) { voice = model.voice; self.model = model }
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(voice.participants) { participant in
                let name = voice.isSelf(participantID: participant.id) ? (model.account?.displayName ?? participant.name) : participant.name
                HStack(spacing: 8) {
                    Avatar(name: name, size: 20, avatarID: participant.avatarId, speaking: voice.speakingParticipants.contains(participant.id))
                        .accessibilityValue(voice.speakingParticipants.contains(participant.id) ? "Speaking" : "")
                    VStack(alignment: .leading, spacing: 1) {
                        Text(name + (voice.isSelf(participantID: participant.id) ? " (you)" : ""))
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
                                #if os(iOS)
                                .frame(minWidth: 44, minHeight: 44)
                                #else
                                .padding(.horizontal, 4).frame(minHeight: 22)
                                #endif
                                .contentShape(Rectangle())
                        }.buttonStyle(.plain).modifier(ControlHover())
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
    #if os(macOS)
    @Environment(\.openSettings) private var openSettings
    /// Closing a dialog opened from this menu returns keyboard focus here, as on web.
    @FocusState private var settingsFocused: Bool
    #else
    @Bindable private var effects = CaperEffects.shared
    #endif
    @State private var inputOptions = false
    @State private var outputOptions = false
    /// The narrowest sidebars leave room for a letter or two ("F…"): the avatar then stands alone.
    @State private var showsName = true
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
                    }.buttonStyle(.plain).modifier(ControlHover())
                    Button { if voice.phase == .connected { CaperEffects.shared.play(.disconnect) }; model.leaveVoice() } label: { CaperIcon(name: "phone-off") }
                        .buttonStyle(SidebarIconButton()).help(voice.phase == .connected ? "Disconnect" : "Cancel")
                        .accessibilityLabel(voice.phase == .connected ? "Leave voice" : "Cancel joining voice")
                }.padding(9).background(CaperTheme.raised).clipShape(RoundedRectangle(cornerRadius: 8))
                    .accessibilityIdentifier("active-voice-context")
            }
            if let error = voice.error {
                HStack(alignment: .top, spacing: 8) {
                    // At least the dismiss button's height, so one line sits centred on its X.
                    Text(error).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
                        .fixedSize(horizontal: false, vertical: true)
                        #if os(iOS)
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                        #else
                        .frame(maxWidth: .infinity, minHeight: 28, alignment: .leading)
                        #endif
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
                    if showsName {
                        Text(identityName).font(CaperTheme.font(13, weight: .medium)).lineLimit(1).truncationMode(.tail)
                    }
                }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                    // The width is what the controls leave, whether or not the name shows.
                    .onGeometryChange(for: Bool.self) { $0.size.width >= 76 } action: { showsName = $0 }
            }.buttonStyle(.plain).modifier(ControlHover())
                .help(identityName)
                // Web: the account's name, else the guest chat identity, with its profile label.
                .accessibilityLabel(model.account == nil ? "Sign in to edit your profile" : "Edit profile for \(identityName)")
                .accessibilityIdentifier("account-profile")
            Button { CaperEffects.shared.toggle(voice.muted); Task { await voice.setMuted(!voice.muted) } } label: {
                CaperIcon(name: voice.muted ? "mic-off" : "mic", size: 20)
                    .foregroundStyle(voice.muted ? CaperTheme.voiceOff : CaperTheme.muted)
            }.buttonStyle(SidebarIconButton()).help(voice.muted ? "Unmute" : "Mute")
                .accessibilityLabel(voice.muted ? "Unmute microphone" : "Mute microphone")
                .accessibilityValue(voice.muted ? "Muted" : "On").accessibilityIdentifier("microphone-toggle")
            Button { inputOptions.toggle() } label: {
                CaperIcon(name: "chevron-down", size: 12)
                    // Red with the microphone while muted, as on web.
                    .foregroundStyle(voice.muted ? CaperTheme.voiceOff : CaperTheme.muted)
                    #if os(iOS)
                    .frame(width: 32, height: 44)
                    #else
                    .frame(width: 14, height: 28)
                    #endif
                    .contentShape(Rectangle())
            }.buttonStyle(.plain).modifier(ControlHover()).help("Input Options").accessibilityLabel("Input Options")
                .popover(isPresented: $inputOptions, arrowEdge: .top) { AccountAudioMenu(voice: voice, input: true) }
            Button { CaperEffects.shared.toggle(voice.deafened); Task { await voice.setDeafened(!voice.deafened) } } label: {
                CaperIcon(name: voice.deafened ? "volume-x" : "headphones", size: 20)
                    .foregroundStyle(voice.deafened ? CaperTheme.voiceOff : CaperTheme.muted)
            }.buttonStyle(SidebarIconButton()).help(voice.deafened ? "Undeafen" : "Deafen")
                .accessibilityLabel(voice.deafened ? "Undeafen audio" : "Deafen audio")
                .accessibilityValue(voice.deafened ? "Deafened" : "On").accessibilityIdentifier("deafen-toggle")
            Button { outputOptions.toggle() } label: {
                CaperIcon(name: "chevron-down", size: 12)
                    .foregroundStyle(voice.deafened ? CaperTheme.voiceOff : CaperTheme.muted)
                    #if os(iOS)
                    .frame(width: 32, height: 44)
                    #else
                    .frame(width: 14, height: 28)
                    #endif
                    .contentShape(Rectangle())
            }.buttonStyle(.plain).modifier(ControlHover()).help("Output Options").accessibilityLabel("Output Options")
                .popover(isPresented: $outputOptions, arrowEdge: .top) { AccountAudioMenu(voice: voice, input: false) }
            Menu {
                #if os(macOS)
                Button("Settings…") { openSettings() }
                    .accessibilityIdentifier("open-settings")
                Divider()
                #else
                Section("Audio settings") {
                    Toggle("Caper sound effects", isOn: $effects.soundsEnabled)
                        .accessibilityIdentifier("sound-effects")
                }
                #endif
                Button("Audio test") { sheet = .audio }
                    .disabled(voice.phase == .leaving)
                if voice.phase == .connected || CaperRuntime.isAudioPreview("audio-statistics") {
                    Button("Connection details") { sheet = .connection }
                }
                if model.account?.debugEnabled == true {
                    Button("Audio diagnostics") { sheet = .diagnostics }
                }
                Button("What’s new") { model.whatsNew.open() }
                    .accessibilityIdentifier("open-whats-new")
                if model.account != nil {
                    Button("Notifications") { sheet = .notifications }
                        .accessibilityIdentifier("open-notifications")
                    Button("Privacy and blocked accounts") { sheet = .privacy }
                        .accessibilityIdentifier("open-privacy")
                    Button("Log out", role: .destructive) { Task { await model.logout() } }
                }
                else { Button("Sign in") { sheet = .login } }
            } label: { CaperIcon(name: "settings", size: 20) }.menuStyle(.borderlessButton).menuIndicator(.hidden)
                #if os(iOS)
                .frame(width: 44, height: 44)
                #else
                .frame(width: 28, height: 28)
                #endif
                .fixedSize()
                .contentShape(Rectangle())
                // The same hover and focus feedback as the microphone and deafen buttons beside it.
                #if os(macOS)
                .focused($settingsFocused)
                .modifier(ControlHover(isFocused: settingsFocused))
                #else
                .modifier(ControlHover())
                #endif
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
        #if os(macOS)
        .onChange(of: sheet?.id) { old, new in
            guard new == nil, let old, ["audio", "connection", "diagnostics", "notifications", "privacy"].contains(old) else { return }
            // Next turn: the workspace is re-enabled once the dialog has gone.
            DispatchQueue.main.async { settingsFocused = true }
        }
        #endif
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
            if let error { Text(error).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error) }
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
var caperResourceBundle: Bundle {
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
    @MainActor public static func image(for avatarID: Int?) -> NSImage? {
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
        if model.selectedChannelID == nil && model.selectedDirectMessageID == nil && model.visit == nil {
            VStack(spacing: 8) {
                // Wider windows already show Browse beside this, where the button did nothing.
                if narrow { Button(action: browse) { Label { Text("Browse spaces") } icon: { CaperIcon(name: "arrow-left", size: 14) } }.buttonStyle(CaperSecondaryButton()) }
                CaperIcon(name: "hash", size: 30).foregroundStyle(CaperTheme.terracottaBright)
                Text(model.spaces.isEmpty ? "Select a direct message" : "No joined channels").font(CaperTheme.font(20, weight: .bold))
                Text(model.spaces.isEmpty ? "Open a conversation from Direct messages." : "Browse public channels or accept a private invitation from the channel list.")
                    .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                    // Centred under the heading, clear of the edges when it wraps.
                    .multilineTextAlignment(.center).padding(.horizontal, 24)
                if model.isOwner { Button("Create channel", action: createChannel).buttonStyle(CaperPrimaryButton()).frame(width: 160).padding(.top, 6) }
            }.frame(maxWidth: .infinity, maxHeight: .infinity).background(CaperTheme.conversation)
        } else { ChatView(model: model, narrow: narrow, browse: browse, membersVisible: membersVisible, toggleMembers: toggleMembers) }
    }
}

/// The timeline's content/viewport heights and whether its end is visible.
private struct TimelinePin: Equatable {
    var height: CGFloat
    var contentHeight: CGFloat
    var atBottom: Bool
    var firstID: String?
    /// The last rows, whose reactions, edits and files grow the end in place.
    var recent: [TimelineEntry]
    /// More than a screen of short messages.
    static let recentRows = 20
    /// `geometry` is the scrolled content's; `viewport` names the scroll view's space.
    init(_ geometry: GeometryProxy, viewport: String, firstID: String?, recent: [TimelineEntry]) {
        let visible = geometry.bounds(of: .named(viewport)) ?? CGRect(origin: .zero, size: geometry.size)
        height = visible.height
        contentHeight = geometry.size.height
        atBottom = visible.maxY >= geometry.size.height - 24
        self.firstID = firstID
        self.recent = recent
    }
}

/// The timeline's last `TimelinePin`. A plain reference box, so scrolling
/// never re-renders the conversation.
private final class TimelinePinBox {
    private var last: TimelinePin?
    /// Whether the reader was last seen at the end; true before the first measurement.
    var atBottom: Bool { last?.atBottom ?? true }
    /// Keep a bottom reader pinned when reactions grow or the viewport shrinks.
    /// A changed first row means history was prepended/replaced, not tail growth.
    /// Growth that leaves the end without any recent row changing is LazyVStack
    /// measuring rows as the reader starts scrolling up; that must not snap back.
    func needsReveal(to pin: TimelinePin) -> Bool {
        defer { last = pin }
        guard let last else { return false }
        return last.atBottom && last.firstID == pin.firstID &&
            (pin.height < last.height || (pin.contentHeight > last.contentHeight && (pin.atBottom || pin.recent != last.recent)))
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
    @AppStorage("caper.threadPanelWidth") private var threadWidth = 340.0
    @State private var threadDragStart: Double?
    @State private var threadDragWidth: Double?
    @FocusState private var threadResizeFocused: Bool
    /// Web shows Connecting…/Offline only after a second without the gateway.
    @State private var showConnectionStatus = false
    /// The channel whose Join is running. Another channel's preview offers its own
    /// Join, and only the channel still open shows a failure.
    @State private var joiningChannelID: String?
    @State private var joinError: String?
    private var joining: Bool { joiningChannelID != nil && joiningChannelID == model.selectedChannelID }
    // A lazy message row can leave the viewport when the keyboard appears or
    // live messages arrive. Keep the picker presenter and target outside it.
    @State private var reactionMessage: ChatMessage?
    @State private var showingEmojiPicker = false
    /// Who reacted: opened by holding a chip (iOS) or its VoiceOver action.
    @State private var reactorsTarget: ReactorsTarget?
    /// The person card for a clicked `@mention` pill: a popover on macOS, a
    /// sheet on iPhone. Opening another pill replaces it.
    @State private var mentionCard: MentionCardTarget?
    @State private var mentionPointer = MentionPointer()
    /// Files are being dragged over the conversation.
    @State private var fileDropTargeted = false
    @State private var fileDropTarget: ChatModel.AttachmentTarget?

    /// The signed-in person, also while previewing without a chat session.
    private var viewerID: String? { chat.currentAuthor?.id ?? model.account?.id }

    private var reactorContext: ReactorContext {
        ReactorContext(viewerID: viewerID, sheetOpen: reactorsTarget != nil) { messageID, emoji in
            guard !SwipeTapGuard.swiping else { return }
            reactionMessage = nil; showingEmojiPicker = false
            reactorsTarget = ReactorsTarget(messageID: messageID, emoji: emoji)
        }
    }
    @StateObject private var composerAutocomplete = ComposerAutocompleteController()
    @State private var showingPins = false
    /// The row that was first before older history loaded: kept in place once it arrives.
    @State private var olderAnchor: String?
    @State private var timelinePin = TimelinePinBox()
    @State private var arrivals = MessageArrivals()
    /// Block confirmation, from the DM header, the request bar or message actions.
    @State private var blockTarget: BlockTarget?
    /// Blocked runs shown in place (in memory only).
    @State private var revealedBlocked: Set<String> = []

    /// Channels offer the member list; phones only once the channel is joined.
    /// A feedback visit has none.
    private var membersAvailable: Bool {
        model.selectedDirectMessageID == nil && model.visit == nil && !(narrow && model.previewingChannel)
    }
    /// The open channel once joined: its header carries voice. Previews and DMs have none.
    private var headerVoiceChannel: Channel? {
        // A visited space channel joins voice from the same header; DMs have none.
        if model.visit != nil { return model.visitChannel.flatMap { model.isVisitVoice($0) ? $0 : nil } }
        guard model.selectedDirectMessageID == nil, let channel = model.selectedChannel, channel.joined else { return nil }
        return channel
    }
    /// The open 1:1 DM, not personal notes.
    private var peerConversation: DirectMessageConversation? {
        model.selectedDirectMessage.flatMap { $0.peer.id == model.account?.id ? nil : $0 }
    }
    private var headerTitle: String {
        // A visit reads "#plans · Studio" or "Alice & Bob", from the visited history.
        if let visit = model.visit { return FeedbackInbox.title(channelName: chat.channelName, spaceName: chat.spaceName, direct: visit.direct) }
        return model.selectedDirectMessageID == nil ? "# \(chat.channelName.lowercased())" : chat.channelName
    }
    private var headerTitleLabel: String {
        if model.visit != nil { return "Visiting \(headerTitle)" }
        return model.selectedDirectMessageID == nil ? "# \(chat.channelName.lowercased())" : "Direct message with \(chat.channelName)"
    }
    /// The space's member count, where the member list applies to a joined channel.
    private var memberCount: Int? {
        guard membersAvailable, headerVoiceChannel != nil, let detail = model.detail, detail.space.demo != true else { return nil }
        return detail.members.count
    }
    /// Web's headerSubtitle under a phone's title: "N members · M in voice", omitting what does not apply.
    private var headerSubtitle: String? {
        var parts: [String] = []
        if let count = memberCount { parts.append("\(count) \(count == 1 ? "member" : "members")") }
        if let channel = headerVoiceChannel {
            let inVoice = voiceOccupants(model, channelID: channel.id).count
            if inVoice > 0 { parts.append("\(inVoice) in voice") }
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
    /// Phones: the name is the conversation's menu trigger, as in Slack, holding
    /// Pins, Members and (in a 1:1 DM) Block. It fills the width the voice group leaves.
    private var titleMenu: some View {
        let direct = model.selectedDirectMessageID != nil
        let membersTitle = membersVisible ? "Hide members" : "Members"
        let membersLabel: String = memberCount.map { "\(membersTitle) · \($0)" } ?? membersTitle
        return Menu {
            Button { showingPins = true } label: { Label { Text("Pins") } icon: { caperMenuIcon("pin") } }
            if membersAvailable {
                Button(action: toggleMembers) {
                    Label { Text(membersLabel) } icon: { caperMenuIcon("users") }
                }
            }
            if let conversation = peerConversation {
                Divider()
                blockMenuItem(conversation)
            }
        } label: {
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    Text(headerTitle).font(CaperTheme.font(14, weight: .bold)).foregroundStyle(CaperTheme.text).lineLimit(1)
                    CaperIcon(name: "chevron-down", size: 14).foregroundStyle(CaperTheme.muted)
                }
                if let subtitle = headerSubtitle {
                    Text(subtitle).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                }
            }
            .padding(.horizontal, 6)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .contentShape(Rectangle())
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden)
        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        .modifier(ControlHover())
        .accessibilityLabel(model.visit != nil ? "\(headerTitle) options"
                            : "\(direct ? "" : "#")\(direct ? chat.channelName : chat.channelName.lowercased()) options")
        .accessibilityIdentifier("channel-title-menu")
        // Web keeps its visually hidden heading beside the menu: the conversation's
        // name for assistive technology, as on wider layouts.
        .background {
            Text(headerTitleLabel).font(CaperTheme.font(14, weight: .bold)).lineLimit(1)
                .foregroundStyle(Color.clear)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
                .allowsHitTesting(false)
                .accessibilityAddTraits(.isHeader)
                .accessibilityLabel(headerTitleLabel)
                .accessibilityIdentifier("selected-channel-name")
        }
    }
    private var timelineEntries: [TimelineEntry] {
        BlockedMessages.entries(chat.displayedChannelMessages, blocked: chat.blockedAuthorIDs, viewerID: viewerID, revealed: revealedBlocked)
    }
    private func channelRow(_ message: ChatMessage, grouping: MessageGrouping.Layout) -> some View {
        MessageRow(message: message, chat: chat, currentUserID: viewerID, reactors: reactorContext, mentionCards: mentionCards,
                   grouped: grouping.grouped.contains(message.id), continued: grouping.continued.contains(message.id),
                   requestBlock: { blockTarget = BlockTarget(author: $0) }) {
            // Holding a chip opens who reacted, not message actions; a swipe
            // that paused before moving opens neither.
            guard reactorsTarget == nil, !SwipeTapGuard.swiping else { return }
            showingEmojiPicker = false
            reactionMessage = message
        }
        .modifier(Entrance.message(arrivals.claim(message.clientMessageId)))
    }
    @ViewBuilder private func blockMenuItem(_ conversation: DirectMessageConversation) -> some View {
        if conversation.blocked || model.blockedIDs.contains(conversation.peer.id) {
            Button("Unblock @\(conversation.peer.username)") { unblock(conversation.peer.id) }
                .accessibilityIdentifier("dm-unblock")
        } else {
            Button("Block @\(conversation.peer.username)…", role: .destructive) { blockTarget = BlockTarget(peer: conversation.peer) }
                .accessibilityIdentifier("dm-block")
        }
    }
    private func unblock(_ accountID: String) {
        Task {
            do { try await model.unblock(accountID: accountID) }
            catch { chat.error = FriendlyError.message(for: error) }
        }
    }

    private var mentionCards: MentionCardContext {
        // A swipe that starts on an avatar, a name or a pill lifts on it too.
        MentionCardContext(pointer: mentionPointer, name: { model.mentionPerson($0, viewerID: viewerID).title }, open: { pill in
            guard !SwipeTapGuard.swiping else { return }
            reactionMessage = nil; showingEmojiPicker = false; reactorsTarget = nil
            mentionCard = MentionCardTarget(pill: pill, pointer: mentionPointer.location)
        }, openAuthor: { author in
            guard !SwipeTapGuard.swiping else { return }
            reactionMessage = nil; showingEmojiPicker = false; reactorsTarget = nil
            mentionCard = MentionCardTarget(author: author, pointer: mentionPointer.location)
        })
    }

    /// Reveal in the same update that clears the composer. Content geometry
    /// repeats this after lazy history resolves, without an extra task/frame.
    private func revealPending(_ proxy: ScrollViewProxy) {
        guard let pending = chat.pendingMessage, pending.threadRootId == nil else { return }
        proxy.scrollTo("pending-\(pending.id)", anchor: .bottom)
    }
    /// The newest message (or the pending send) at the bottom, as when one arrives.
    private func revealLatest(_ proxy: ScrollViewProxy) {
        if chat.pendingMessage != nil { revealPending(proxy) }
        else if !chat.hasNewer, chat.focusedMessageID == nil, let id = chat.channelMessages.last?.id { proxy.scrollTo(id, anchor: .bottom) }
    }
    /// Older history goes above the reader's place. Remember the first row so
    /// the page that arrives cannot push it (and what they were reading) away.
    private func loadOlder() {
        let first = chat.channelMessages.first?.id
        olderAnchor = timelineEntries.first?.scrollID
        Task {
            await chat.loadOlder()
            // Nothing was added (an error, or a load already running): forget it.
            if chat.channelMessages.first?.id == first { olderAnchor = nil }
        }
    }
    init(model: AppModel, narrow: Bool, browse: @escaping () -> Void, membersVisible: Bool, toggleMembers: @escaping () -> Void) {
        self.model = model; chat = model.chat; voice = model.voice; self.narrow = narrow; self.browse = browse
        self.membersVisible = membersVisible; self.toggleMembers = toggleMembers
    }
    var body: some View {
        Group {
        GeometryReader { geometry in
            let maximum = Double(min(600, geometry.size.width - min(300, geometry.size.width / 2)))
            HStack(spacing: 0) {
                #if os(iOS)
                channelBody
                #else
                if !narrow || chat.threadRootID == nil { channelBody }
                #endif
                if chat.threadRootID != nil {
                    if narrow {
                        #if os(macOS)
                        threadContent.frame(maxWidth: .infinity).modifier(Entrance.panel)
                        #endif
                    } else { resizableThread(maximum: maximum) }
                }
            }
        }
        #if os(iOS)
        .fullScreenCover(isPresented: Binding(get: { narrow && chat.threadRootID != nil }, set: { if !$0 { chat.closeThread() } })) {
            threadContent
        }
        #else
        // Web: Escape closes the thread unless a picker or dialog is open. Sheets,
        // popovers (Pins included) and menus are their own windows and keep
        // Escape; the composer keeps it for its suggestions.
        .onExitCommand {
            guard chat.threadRootID != nil, !showingPins, reactionMessage == nil, reactorsTarget == nil, mentionCard == nil else { return }
            chat.closeThread()
        }
        #endif
        }
        .sheet(item: $chat.forwardTarget) { message in ForwardPickerView(chat: chat, message: message) }
        .sheet(item: $chat.forwardConversationTarget) { message in ForwardConversationView(chat: chat, messageID: message.id) }
    }
    private var threadContent: some View {
        NativeThreadView(chat: chat, model: model, viewerID: viewerID, mentions: mentionSource)
    }
    private func resizableThread(maximum: Double) -> some View {
        let minimum = min(300, maximum)
        let width = min(maximum, max(minimum, threadDragWidth ?? threadWidth))
        func resize(_ value: Double) {
            threadWidth = max(300, min(maximum, max(minimum, value)))
        }
        return threadContent.frame(width: width)
            .overlay(alignment: .leading) {
                Color.clear.frame(width: 8).frame(maxHeight: .infinity)
                    .contentShape(Rectangle())
                    .overlay(alignment: .leading) {
                        if threadResizeFocused { Rectangle().fill(CaperTheme.terracotta).frame(width: 2) }
                    }
                    .modifier(ResizeCursor())
                    .highPriorityGesture(DragGesture(minimumDistance: 1, coordinateSpace: .global).onChanged { value in
                        threadResizeFocused = true
                        if threadDragStart == nil { threadDragStart = width }
                        threadDragWidth = min(maximum, max(minimum, (threadDragStart ?? width) - Double(value.translation.width)))
                    }.onEnded { _ in
                        if let value = threadDragWidth { resize(value) }
                        threadDragWidth = nil; threadDragStart = nil
                    })
                    .simultaneousGesture(TapGesture(count: 2).onEnded { resize(340); threadResizeFocused = true })
                    .onTapGesture { threadResizeFocused = true }
                    .focusable().focusEffectDisabled().focused($threadResizeFocused)
                    .accessibilityRepresentation {
                        if maximum > minimum {
                            Slider(value: Binding(get: { width }, set: { resize($0) }), in: minimum...maximum, step: 10)
                        } else { Text("\(Int(width)) pixels") }
                    }
                    .accessibilityLabel("Thread width")
                    .accessibilityIdentifier("thread-panel-resize")
                    .accessibilityValue("\(Int(width)) pixels")
                    .accessibilityHint("Drag to resize. Left widens and Right narrows by 10 pixels; Home and End select the bounds. Double-click resets.")
                    .onKeyPress { press in
                        switch press.key {
                        case .leftArrow: resize(width + 10)
                        case .rightArrow: resize(width - 10)
                        case .home: resize(minimum)
                        case .end: resize(maximum)
                        default: return .ignored
                        }
                        return .handled
                    }
            }
            .modifier(Entrance.panel)
    }
    private var channelBody: some View {
        VStack(spacing: 0) {
            HStack(spacing: narrow ? 5 : 10) {
                if narrow {
                    Button(action: SwipeTapGuard.unlessSwiping(browse)) { CaperIcon(name: "arrow-left", size: 20).frame(width: 44, height: 44).contentShape(Rectangle()) }
                        .buttonStyle(.plain).foregroundStyle(CaperTheme.muted).modifier(ControlHover()).accessibilityLabel("Back to Browse")
                    // Phones: the name opens the conversation's menu, as in Slack; Pins,
                    // Members and a 1:1 DM's Block move there.
                    titleMenu
                } else {
                    Text(headerTitle).font(CaperTheme.font(14, weight: .medium)).lineLimit(1)
                        // A long name truncates only once the voice faces have compacted.
                        .layoutPriority(1)
                        .accessibilityLabel(headerTitleLabel)
                        .accessibilityIdentifier("selected-channel-name")
                    Spacer()
                }
                // Before the buttons, so they stay put when the status comes and goes.
                if chat.liveState != .connected && showConnectionStatus {
                    Text(chat.liveState == .disconnected ? "Offline" : "Connecting…").font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.muted)
                        .lineLimit(1).fixedSize()
                        .accessibilityIdentifier("chat-connection-status")
                }
                // Web joins voice from here on every device; touch has no row shortcut.
                // Wider headers set it apart from the Pins and Members icons with a rule.
                if let channel = headerVoiceChannel {
                    HeaderVoice(model: model, channel: channel, narrow: narrow)
                    if !narrow {
                        Rectangle().fill(CaperTheme.border).frame(width: 1, height: 24)
                            .padding(.horizontal, 2).accessibilityHidden(true)
                    }
                } else if narrow && inVoiceCall(voice) {
                    // A DM or preview has no voice of its own; phones still offer the call's mute, deafen and Leave.
                    HStack(spacing: 2) {
                        HeaderMuteButton(voice: voice)
                        HeaderDeafenButton(voice: voice)
                        HeaderLeaveButton(model: model, compact: true)
                    }.modifier(CompactVoiceGroup())
                }
                if !narrow {
                    Button { showingPins = true } label: { CaperIcon(name: "pin", size: 20) }
                        .buttonStyle(SidebarIconButton()).help("Pins").accessibilityLabel("Pins")
                        .accessibilityIdentifier("channel-pins")
                    // Right of Pins, as on web.
                    if membersAvailable {
                        Button(action: toggleMembers) { CaperIcon(name: "users", size: 20) }
                            .buttonStyle(SidebarIconButton()).help(membersVisible ? "Hide member list" : "Show member list")
                            .accessibilityLabel(membersVisible ? "Hide member list" : "Show member list")
                    }
                    if let conversation = peerConversation {
                        Menu { blockMenuItem(conversation) } label: { CaperIcon(name: "ellipsis", size: 20) }
                            .menuStyle(.borderlessButton).menuIndicator(.hidden).frame(width: 28, height: 28)
                            .contentShape(Rectangle()).modifier(ControlHover())
                            .help("Conversation options").accessibilityLabel("Conversation options")
                            .accessibilityIdentifier("dm-options")
                    }
                }
                // Wider layouts close a visit back to the conversation before it;
                // phones go back to Browse, where the inbox stays open.
                if !narrow, model.visit != nil {
                    Button { Task { await model.closeVisit() } } label: { CaperIcon(name: "x", size: 20) }
                        .buttonStyle(SidebarIconButton()).help("Close").accessibilityLabel("Close conversation")
                        .accessibilityIdentifier("feedback-visit-close")
                }
            }
            .padding(.leading, narrow ? 13 : 18).padding(.trailing, narrow ? 10 : 18).frame(height: 50)
                .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
                .modifier(BrowseSwipe(open: false, enabled: narrow && !membersVisible && !showingPins && reactionMessage == nil && reactorsTarget == nil && !showingEmojiPicker && mentionCard == nil, navigate: browse))
                .modifier(MembersSwipe(open: membersVisible, enabled: narrow && membersAvailable && !showingPins && reactionMessage == nil && reactorsTarget == nil && !showingEmojiPicker && mentionCard == nil, navigate: toggleMembers))
                #if os(macOS)
                // Use the fixed header as the native anchor. A point inside the
                // timeline can resolve to a lazy row that live delivery removes.
                .popover(item: $reactionMessage, attachmentAnchor: .point(.bottom), arrowEdge: .top) { message in
                    ReactionPicker { emoji in
                        reactionMessage = nil
                        Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: true) }
                    }
                }
                #endif

            ScrollViewReader { proxy in
                ScrollView(showsIndicators: false) {
                    let entries = timelineEntries
                    let recentRows = Array(entries.suffix(TimelinePin.recentRows))
                    // The channel's own pending message; a thread reply's shows in the thread.
                    let pending: PendingMessage? = chat.pendingMessage.flatMap { $0.threadRootId == nil ? $0 : nil }
                    let grouping = MessageGrouping.layout(entries, inThread: false, pending: pending, author: chat.currentAuthor)
                    // Before any row is built, so rows see this render's arrivals.
                    let _ = arrivals.observe("channel", key: chat.channelID ?? "",
                                             ids: chat.channelMessages.map(\.clientMessageId) + (pending.map { [$0.id] } ?? []),
                                             live: !chat.loading && !chat.loadFailed && !chat.hasNewer && !chat.loadingNewer)
                    VStack(spacing: 0) {
                        LazyVStack(spacing: 0) {
                            if !chat.loadFailed { HStack(spacing: 6) {
                                if chat.olderError != nil {
                                    Text("Couldn’t load older messages.")
                                    Button("Retry", action: SwipeTapGuard.unlessSwiping(loadOlder)).disabled(chat.loadingOlder)
                                        .accessibilityIdentifier("load-older-messages")
                                } else if chat.hasMore {
                                    Button(chat.loadingOlder ? "Loading…" : "Load older messages", action: SwipeTapGuard.unlessSwiping(loadOlder))
                                        .disabled(chat.loadingOlder)
                                        .accessibilityIdentifier("load-older-messages")
                                }
                                else { Text("Beginning of conversation") }
                            }.font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.muted).frame(height: 44) }
                            if chat.loading && chat.messages.isEmpty {
                                MessageSkeleton()
                            }
                            ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
                                VStack(spacing: 0) {
                                    if index == 0 || !ChatDateDivider.sameLocalDay(entries[index - 1].lastCreatedAt, entry.firstCreatedAt) {
                                        ChatDateDivider(createdAt: entry.firstCreatedAt)
                                    }
                                    switch entry {
                                    case let .message(message): channelRow(message, grouping: grouping)
                                    case let .blocked(run):
                                        VStack(spacing: 0) {
                                            BlockedRunRow(run: run) { revealedBlocked = BlockedMessages.toggling(run, in: revealedBlocked) }
                                            if run.revealed { ForEach(run.messages) { channelRow($0, grouping: grouping) } }
                                        }
                                        .background(run.revealed ? CaperTheme.surface : .clear)
                                        .overlay(alignment: .leading) {
                                            if run.revealed { Rectangle().fill(CaperTheme.border).frame(width: 2) }
                                        }
                                    }
                                }.id(entry.scrollID)
                            }
                        }
                        // Keep the scroll target eager even when lazy history
                        // has not yet resolved the heights of preceding rows.
                        if let pending {
                            VStack(spacing: 0) {
                                if chat.channelMessages.last.map({ ChatDateDivider.sameLocalDay($0.createdAt, pending.createdAt) }) != true {
                                    ChatDateDivider(createdAt: pending.createdAt)
                                }
                                PendingMessageRow(pending: pending, author: chat.currentAuthor, error: chat.error,
                                                  rejected: chat.sendRejected, canEdit: chat.draft.isEmpty && chat.attachmentDrafts.isEmpty, chat: chat, uploadFailed: chat.sendUploadFailed, grouped: grouping.pendingGrouped,
                                                  retry: { Task { await chat.send() } },
                                                  edit: { _ = chat.discardRejected(edit: true) },
                                                  dismiss: { _ = chat.discardRejected() })
                                    .modifier(Entrance.message(arrivals.claim(pending.id)))
                            }.id("pending-\(pending.id)")
                                .onAppear { revealPending(proxy) }
                        }
                        if chat.loadFailed, let error = chat.error {
                            // Web's failed first load: the error with Try again, in place of the conversation.
                            VStack(spacing: 10) {
                                Text(error).font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).multilineTextAlignment(.center)
                                Button("Try again") { if !SwipeTapGuard.swiping { Task { await chat.retryLoad() } } }.buttonStyle(CaperSecondaryButton())
                                    .accessibilityIdentifier("chat-try-again")
                            }.padding(.top, 80).padding(.horizontal, 24)
                        } else if chat.messages.isEmpty && !chat.loading && chat.pendingMessage == nil {
                            VStack(spacing: 7) {
                                if let conversation = model.selectedDirectMessage, conversation.peer.id == model.account?.id {
                                    Text("You can message yourself here to keep notes, reminders, and ideas.")
                                        .font(CaperTheme.font(14, weight: .medium)).foregroundStyle(CaperTheme.muted).multilineTextAlignment(.center)
                                } else {
                                    Text("No messages yet.").font(CaperTheme.font(14, weight: .medium))
                                    Text(model.selectedDirectMessageID == nil ? "Start the conversation in #\(chat.channelName.lowercased())." : FeedbackInbox.emptyDirectNote(peer: model.selectedDirectMessage?.peer, name: chat.channelName))
                                        .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted).multilineTextAlignment(.center).padding(.horizontal, 24)
                                }
                            }.padding(.top, 80)
                        }
                    }
                    // Preceding history can move the pending row without
                    // changing that row's own height; observe the whole extent.
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { _ in revealPending(proxy) }
                    // Reactions grow existing rows; a draft or keyboard shrinks
                    // the viewport. Both retain a reader already at the bottom.
                    .onGeometryChange(for: TimelinePin.self) { TimelinePin($0, viewport: MentionCard.timelineSpace, firstID: chat.channelMessages.first?.id, recent: recentRows) } action: { pin in
                        if timelinePin.needsReveal(to: pin) { revealLatest(proxy) }
                    }
                }
                #if os(iOS)
                .scrollDismissesKeyboard(.immediately)
                #endif
                .accessibilityIdentifier("chat-timeline")
                // Pills report the pointer in this space; the popover anchors in it too.
                .coordinateSpace(.named(MentionCard.timelineSpace))
                .modifier(MentionCardHost(target: $mentionCard, model: model, viewerID: viewerID))
                .modifier(BrowseSwipe(open: false, enabled: narrow && !membersVisible && reactionMessage == nil && reactorsTarget == nil && !showingEmojiPicker && mentionCard == nil, navigate: browse))
                // Right opens Browse; left opens Members, from anywhere in the timeline.
                .modifier(MembersSwipe(open: false, enabled: narrow && membersAvailable && !membersVisible && reactionMessage == nil && reactorsTarget == nil && !showingEmojiPicker && mentionCard == nil, navigate: toggleMembers))
                #if os(macOS)
                // Web: End in the message list jumps to the latest message.
                .focusable().focusEffectDisabled()
                .onKeyPress(.end) {
                    guard let id = chat.channelMessages.last?.id else { return .ignored }
                    proxy.scrollTo(id, anchor: .bottom)
                    return .handled
                }
                #endif
                .onChange(of: chat.channelMessages.last?.id) { old, id in
                    if chat.pendingMessage != nil { revealPending(proxy) }
                    else if chat.focusedMessageID == nil, let id {
                        // As on web, only local sends (revealPending) override the reader's
                        // place: a new message follows a reader already at the end, while a
                        // new conversation or replaced history (the old last row is gone) opens there.
                        let replaced = old.map { old in !chat.channelMessages.contains { $0.id == old } } ?? true
                        if replaced || timelinePin.atBottom { proxy.scrollTo(id, anchor: .bottom) }
                    }
                }
                // Older history arrived above: the row that was first stays at the top.
                .onChange(of: chat.channelMessages.first?.id) { _, _ in
                    guard let anchor = olderAnchor else { return }
                    olderAnchor = nil
                    proxy.scrollTo(anchor, anchor: .top)
                }
                .onChange(of: chat.focusRevision) { _, _ in
                    // A jump replaces the history window; it is not an older page.
                    olderAnchor = nil
                    if let id = chat.focusedMessageID, chat.channelMessages.contains(where: { $0.id == id }) {
                        revealedBlocked.insert(id)
                        Task { @MainActor in
                            await Task.yield()
                            if chat.focusedMessageID == id { proxy.scrollTo(id, anchor: .center) }
                        }
                    }
                }
                .onChange(of: chat.pendingMessage?.id, initial: true) { _, _ in revealPending(proxy) }
                // A rejection adds the "Not sent" line with Edit and Dismiss after
                // the first scroll, so the row grows below the viewport; reveal it again.
                .onChange(of: chat.sendRejected) { _, _ in revealPending(proxy) }
                .onChange(of: chat.error) { _, _ in revealPending(proxy) }
            }

            if chat.hasNewer {
                HStack {
                    Button(chat.loadingNewer ? "Loading…" : "Load newer messages") { Task { await chat.loadNewer() } }.disabled(chat.loadingNewer)
                    Button("Back to latest") { Task { await chat.retryLoad() } }
                }.font(CaperTheme.font(11)).padding(10)
            }
            if let sessionError = chat.sessionError {
                HStack(spacing: 8) {
                    Text(sessionError).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
                    Button("Retry session") { Task { await chat.retrySession() } }.buttonStyle(.plain)
                        .font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.text).modifier(ControlHover())
                    Spacer()
                }.padding(.horizontal, 18)
            }
            if let error = chat.error, chat.pendingMessage == nil, !chat.loadFailed {
                Text(error).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error).frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18)
            }
            // Phones show the conversation alone, without the account bar's dock,
            // so voice permission and failure errors come with it.
            if narrow, let error = voice.error {
                HStack(alignment: .top, spacing: 8) {
                    Text(error).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
                        .fixedSize(horizontal: false, vertical: true)
                        #if os(iOS)
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                        #else
                        .frame(maxWidth: .infinity, minHeight: 28, alignment: .leading)
                        #endif
                    Button { voice.error = nil } label: { CaperIcon(name: "x", size: 14) }
                        .buttonStyle(SidebarIconButton()).accessibilityLabel("Dismiss voice error")
                }.padding(9).background(CaperTheme.raised).clipShape(RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                    .padding(.horizontal, 18).padding(.top, 8)
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("voice-error-inline")
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
                        joiningChannelID = channel.id; joinError = nil
                        Task {
                            do { try await model.joinChannel(channel) }
                            catch { if model.selectedChannelID == channel.id { joinError = FriendlyError.message(for: error) } }
                            if joiningChannelID == channel.id { joiningChannelID = nil }
                        }
                    }.buttonStyle(CaperPrimaryButton()).disabled(joining)
                }
                if let joinError { Text(joinError).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error) }
                }.padding(12)
            } else if let request = model.selectedDirectMessage, request.status == .incoming {
                // Requests are read-only: Accept, Decline or Block replace the composer.
                MessageRequestBar(model: model, conversation: request) { blockTarget = BlockTarget(peer: request.peer) }
            } else if let conversation = peerConversation, conversation.blocked {
                BlockedConversationBar(conversation: conversation) { unblock(conversation.peer.id) }
            } else { VStack(alignment: .leading, spacing: 8) {
            if !chat.typingNames.isEmpty {
                HStack(spacing: 7) {
                    TypingDots()
                    Text(typingLabel).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                    Spacer()
                }.frame(height: 20)
            }
            if let reference = chat.replyTargets["channel"] { NativeReplyDraft(reference: reference, chat: chat, root: "channel") }
            if let notice = chat.attachmentNotice {
                Text(notice).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
            }
            AttachmentDraftsView(chat: chat)
            HStack(alignment: .bottom, spacing: 8) {
                VStack(spacing: 6) {
                    if let conversation = peerConversation, conversation.status == .outgoing {
                        Text(MessageRequests.waitingNotice(for: conversation.peer))
                            .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .accessibilityIdentifier("message-request-waiting")
                    }
                    if let root = chat.pendingMessage?.threadRootId, !chat.sending,
                       chat.error != nil || chat.sendRejected, chat.threadRootID != root {
                        HStack(spacing: 8) {
                            Text(chat.sendRejected ? "A thread reply wasn’t sent." : "A thread reply couldn’t be confirmed.")
                                .foregroundStyle(CaperTheme.error)
                            Button("Review reply") { Task { await chat.openThread(root) } }
                        }.font(CaperTheme.font(11))
                    }
                    #if os(iOS)
                    ComposerSuggestionsView(controller: composerAutocomplete)
                    #endif
                    HStack(alignment: .bottom, spacing: 0) {
                        AttachmentPickerButton(chat: chat).padding(.leading, 4)
                        ZStack(alignment: .topLeading) {
                            if chat.draft.isEmpty {
                                Text(composerPlaceholder).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted)
                                    .padding(.leading, 11).padding(.top, 12).allowsHitTesting(false).accessibilityHidden(true)
                            }
                            messageComposer
                        }
                        .frame(minHeight: 42, maxHeight: 174)
                        // Measure the text view instead of filling the 174-point maximum.
                        .fixedSize(horizontal: false, vertical: true)
                        .onChange(of: chat.draft) { _, value in chat.draftChanged(value) }
                        #if os(iOS)
                        .overlay(alignment: .topLeading) { ParityKeyboardProbe() }
                        #endif
                        ComposerEmojiButton(controller: composerAutocomplete).padding(.trailing, 2)
                    }
                    .background(CaperTheme.composer).clipShape(RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                    #if os(macOS)
                    // The conversation draws suggestions above this box (see ComposerSuggestionsOverlay).
                    .anchorPreference(key: ChatComposerAnchorKey.self, value: .bounds) { $0 }
                    #endif
                }
                Button { Task { await chat.send() } } label: {
                    CaperIcon(name: "send", size: 16)
                        // Size the actual label, not only its styled background: clicks
                        // beside the glyph must submit too, especially on macOS.
                        .frame(width: 42, height: 42)
                        .contentShape(Rectangle())
                }
                .buttonStyle(PrimaryIconButton())
                .disabled(!chat.canSubmit || chat.pendingMessage?.threadRootId != nil)
                .help(chat.sending ? "Sending…" : chat.sendRejected ? "Edit or dismiss the rejected message before sending another." : "Send message")
                .accessibilityLabel("Send message")
                .accessibilityValue(chat.sending ? "Sending" : "")
                .accessibilityIdentifier("send-message-button")
            }
            }
            }
            }.padding(.horizontal, 18).padding(.top, 12)
            // Keep breathing room above the keyboard/home-indicator safe area.
            .padding(.bottom, 16)
            if chat.draft.unicodeScalars.count >= 3000 {
                Text("\(chat.draft.unicodeScalars.count.formatted()) / 4,000").font(CaperTheme.font(10)).monospacedDigit().foregroundStyle(counterTone).padding(.bottom, 6)
            }
        }.background {
            // Paint behind iOS's rounded keyboard corners; only the background
            // ignores the safe area, so the controls still avoid the keyboard.
            CaperTheme.conversation.ignoresSafeArea()
        }
            // Uploads are optional server configuration; recheck per conversation.
            .task(id: model.selectedDirectMessageID ?? model.selectedChannelID ?? model.visit?.channelID) { await chat.checkUploadAvailability() }
            // Finder, Files, Photos and other apps (iPad split view and Slide Over).
            // addAttachments keeps the first files that fit and removes the rest.
            .dropDestination(for: DroppedAttachment.self) { items, _ in
                guard let target = fileDropTarget ?? chat.attachmentTarget(), !items.isEmpty else {
                    items.forEach { AttachmentStaging.remove($0.file.url) }
                    return false
                }
                chat.addAttachments(items.map(\.file), to: target)
                fileDropTarget = nil
                return true
            } isTargeted: {
                fileDropTargeted = $0
                if $0 { fileDropTarget = chat.attachmentTarget() }
            }
            .overlay {
                if fileDropTargeted && chat.canAttach {
                    let text = AttachmentPolicy.dropText(channel: chat.channelName, direct: model.selectedDirectMessageID != nil || model.visit?.direct == true,
                                                         attached: chat.attachmentDrafts.count)
                    FileDropOverlay(title: text.title, detail: text.detail)
                }
            }
            #if os(macOS)
            .modifier(ComposerSuggestionsOverlay<ChatComposerAnchorKey>(controller: composerAutocomplete))
            // Drawn above the whole conversation so the timeline's clipping
            // and later rows never cover a hovered chip's tooltip.
            .overlayPreferenceValue(ReactionTooltipKey.self) { tooltip in
                GeometryReader { proxy in
                    if let tooltip {
                        let chip = proxy[tooltip.anchor]
                        ReactionTooltipBubble(emoji: tooltip.emoji, text: tooltip.text)
                            .modifier(Entrance.anchored)
                            // Start at the chip, kept 8 points inside the conversation.
                            .alignmentGuide(.leading) { size in
                                let rightmost: CGFloat = max(8, proxy.size.width - size.width - 8)
                                return -min(max(8, chip.minX), rightmost)
                            }
                            // Above the chip; below it when there is no room above.
                            .alignmentGuide(.top) { size in
                                let top: CGFloat = chip.minY - 6 - size.height
                                return top >= 0 ? -top : -(chip.maxY + 6)
                            }
                            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                    }
                }
                .allowsHitTesting(false)
            }
            #endif
            .sheet(item: $reactorsTarget) { target in
                ReactorsSheet(chat: chat, messageID: target.messageID, emoji: target.emoji, viewerID: viewerID)
            }
            #if os(iOS)
            .sheet(item: $reactionMessage) { message in
                MessageActionsSheet(message: message, chat: chat, showingEmojiPicker: $showingEmojiPicker,
                                    canReact: chat.canInteract,
                                    canPin: chat.canInteract && !chat.pendingPins.contains(message.id),
                                    togglePin: {
                                        reactionMessage = nil
                                        Task { await chat.setPin(messageID: message.id, active: message.pin == nil) }
                                    },
                                    reply: { reactionMessage = nil; Task { await chat.openThread(message.threadRootId ?? message.id) } },
                                    canForward: chat.canForward,
                                    forward: { reactionMessage = nil; chat.forwardTarget = message },
                                    quickReaction: { emoji in
                                        guard chat.canInteract, let author = chat.currentAuthor,
                                              let current = chat.messages.first(where: { $0.id == message.id }) else { return }
                                        let own = current.reactions?.first(where: { $0.emoji == emoji })?.authorIds.contains(author.id) == true
                                        reactionMessage = nil
                                        Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: !own) }
                                    }, selectReaction: { emoji in
                                        guard chat.canInteract else { return }
                                        reactionMessage = nil
                                        Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: true) }
                                    },
                                    block: BlockedMessages.canBlock(message.author, viewerID: viewerID, blocked: chat.blockedAuthorIDs)
                                        ? { blockTarget = BlockTarget(author: message.author) } : nil,
                                    unblock: BlockedMessages.canUnblock(message.author, viewerID: viewerID, blocked: chat.blockedAuthorIDs)
                                        ? { chat.unblock(message.author) } : nil,
                                    viewReactions: { emoji in reactorContext.show(message.id, emoji) })
            }
            #endif
            #if os(macOS)
            .popover(isPresented: $showingPins, attachmentAnchor: .point(.center), arrowEdge: .top) {
                PinnedMessagesView(chat: chat, model: model, close: { showingPins = false }).frame(width: 470, height: 360)
            }
            #else
            .sheet(isPresented: $showingPins) {
                PinnedMessagesView(chat: chat, model: model, close: { showingPins = false })
            }
            #endif
            .modifier(BlockConfirmation(target: $blockTarget) { target in
                do { try await model.block(target) } catch { chat.error = FriendlyError.message(for: error) }
            })
            .onChange(of: chat.isPreview) { _, preview in
                if preview { reactionMessage = nil; showingEmojiPicker = false }
            }
            .onChange(of: chat.currentAuthor?.id) { _, _ in reactionMessage = nil; showingEmojiPicker = false }
            .onChange(of: model.selectedChannelID) { _, _ in
                reactionMessage = nil; showingEmojiPicker = false; reactorsTarget = nil; showingPins = false; mentionCard = nil; olderAnchor = nil
                joinError = nil
            }
            .onChange(of: model.selectedDirectMessageID) { _, _ in
                reactionMessage = nil; showingEmojiPicker = false; reactorsTarget = nil; showingPins = false; mentionCard = nil; olderAnchor = nil
                joinError = nil
            }
            .onChange(of: model.visit?.channelID) { _, _ in
                reactionMessage = nil; showingEmojiPicker = false; reactorsTarget = nil; showingPins = false; mentionCard = nil; olderAnchor = nil
                joinError = nil
            }
            .task(id: chat.liveState) {
                showConnectionStatus = false
                guard chat.liveState != .connected else { return }
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                showConnectionStatus = true
            }
    }
    /// Web: "Message #general" in a channel, "Message Alice Smith" in a DM
    /// (including your own notes, named for you).
    private var composerPlaceholder: String {
        let direct = model.selectedDirectMessageID != nil || model.visit?.direct == true
        return direct ? "Message \(chat.channelName)" : "Message #\(chat.channelName.lowercased())"
    }
    // Type-check the native editor and its optional callback separately from
    // channelBody's large SwiftUI builder; keep the editor at the same position.
    private var messageComposer: NativeMessageComposer {
        let pasteAttachments: (() -> Bool)?
        if chat.canAttach {
            pasteAttachments = { chat.pasteAttachments() }
        } else {
            pasteAttachments = nil
        }
        return NativeMessageComposer(text: $chat.draft, placeholder: composerPlaceholder,
                                     controller: composerAutocomplete, mentions: mentionSource,
                                     submit: { Task { await chat.send() } },
                                     pasteAttachments: pasteAttachments)
    }
    /// `@` suggestions: the open space's members as `GET /api/spaces/{space}`
    /// returned them (only the specials until loaded), or in a DM everyone from
    /// `GET /api/people` (the DM's other participant until that loads); in a
    /// feedback visit, the reporter and people in the loaded messages. The
    /// signed-in account is never suggested; `@feedback` is offered everywhere.
    private var mentionSource: MentionSource {
        if let visit = model.visit {
            return .visit(reporter: visit.reporter, messages: chat.messages, people: model.people, accountID: viewerID)
        }
        if let id = model.selectedDirectMessageID {
            return .direct(peer: model.directMessages.first { $0.id == id }?.peer, people: model.people, accountID: viewerID)
        }
        return .space(members: model.detail?.members ?? [], excluding: viewerID)
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

private struct NativeThreadView: View {
    @Bindable var chat: ChatModel
    /// For the mention card: names from loaded members/people/DMs, and Message.
    let model: AppModel
    /// The signed-in person, for the mentions-me tint on replies.
    let viewerID: String?
    /// The conversation's `@` candidates, shared with the main composer.
    let mentions: MentionSource
    @StateObject private var composerAutocomplete = ComposerAutocompleteController()
    @State private var fileDropTargeted = false
    @State private var fileDropTarget: ChatModel.AttachmentTarget?
    @State private var reactionMessage: ChatMessage?
    @State private var showingEmojiPicker = false
    @State private var reactorsTarget: ReactorsTarget?
    /// Pills in the thread open their own card, anchored in this panel.
    @State private var mentionCard: MentionCardTarget?
    @State private var mentionPointer = MentionPointer()
    private var mentionCards: MentionCardContext {
        MentionCardContext(pointer: mentionPointer, name: { model.mentionPerson($0, viewerID: viewerID).title }, open: { pill in
            reactionMessage = nil; showingEmojiPicker = false; reactorsTarget = nil
            mentionCard = MentionCardTarget(pill: pill, pointer: mentionPointer.location)
        }, openAuthor: { author in
            reactionMessage = nil; showingEmojiPicker = false; reactorsTarget = nil
            mentionCard = MentionCardTarget(author: author, pointer: mentionPointer.location)
        })
    }
    @State private var blockTarget: BlockTarget?
    @State private var revealedBlocked: Set<String> = []
    /// iOS: the forward picker for a thread message, presented by this view.
    @State private var forwardMessage: ChatMessage?
    /// As in the channel: the first reply before older ones load stays in place.
    @State private var olderAnchor: String?
    @State private var timelinePin = TimelinePinBox()
    @State private var arrivals = MessageArrivals()
    /// Your latest reply sent from this thread: its arrival scrolls to it.
    @State private var localReplyID: String?
    /// Web: an icon send button, leaving the row to the broadcast toggle. Kept
    /// out of the body so the type checker handles it separately.
    private var sendReplyButton: some View {
        Button { Task { await chat.send(inThread: true) } } label: {
            CaperIcon(name: "send", size: 18)
                .frame(width: 42, height: 42)
                .contentShape(Rectangle())
        }
        .buttonStyle(PrimaryIconButton())
        .disabled(chat.threadLoading || chat.pendingMessage != nil || !chat.canSubmit(inThread: true))
        .help("Send reply")
        .accessibilityLabel("Send reply")
    }
    private func entries(_ messages: [ChatMessage]) -> [TimelineEntry] {
        BlockedMessages.entries(messages, blocked: chat.blockedAuthorIDs, viewerID: chat.viewerID, revealed: revealedBlocked)
    }
    private func threadRow(_ message: ChatMessage, grouping: MessageGrouping.Layout) -> some View {
        MessageRow(message: message, chat: chat, currentUserID: viewerID, reactors: reactors, mentionCards: mentionCards, inThread: true,
                   grouped: grouping.grouped.contains(message.id), continued: grouping.continued.contains(message.id),
                   requestBlock: { blockTarget = BlockTarget(author: $0) }) { reactionMessage = message }
            .modifier(Entrance.message(arrivals.claim(message.clientMessageId)))
    }
    /// The root and replies collapse like the channel: blocked runs show "Show".
    /// Replies group among themselves; the root never groups (empty `grouping`).
    @ViewBuilder private func threadEntries(_ rows: [TimelineEntry], grouping: MessageGrouping.Layout = .init()) -> some View {
        ForEach(rows) { entry in
            switch entry {
            case let .message(message): threadRow(message, grouping: grouping).id(message.id)
            case let .blocked(run):
                VStack(spacing: 0) {
                    BlockedRunRow(run: run) { revealedBlocked = BlockedMessages.toggling(run, in: revealedBlocked) }
                    if run.revealed { ForEach(run.messages) { threadRow($0, grouping: grouping) } }
                }
                .background(run.revealed ? CaperTheme.surface : .clear)
                .overlay(alignment: .leading) {
                    if run.revealed { Rectangle().fill(CaperTheme.border).frame(width: 2) }
                }
                .id(entry.scrollID)
            }
        }
    }
    private var replies: [ChatMessage] { chat.displayedThreadMessages }
    private func loadOlderReplies() {
        let first = chat.threadMessages.first?.id
        olderAnchor = entries(replies).first?.scrollID
        Task {
            await chat.loadThread(older: true)
            if chat.threadMessages.first?.id == first { olderAnchor = nil }
        }
    }
    /// DMs are named after the other person, without a channel `#`.
    private var direct: Bool { model.selectedDirectMessageID != nil || model.visit?.direct == true }
    private var reactors: ReactorContext {
        ReactorContext(viewerID: chat.currentAuthor?.id, sheetOpen: reactorsTarget != nil) { messageID, emoji in
            reactorsTarget = ReactorsTarget(messageID: messageID, emoji: emoji)
        }
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 3) { Text("Thread").font(CaperTheme.font(15, weight: .bold)); Text(direct ? "with \(chat.channelName)" : "in #\(chat.channelName)").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).lineLimit(1) }
                Spacer()
                Button { chat.closeThread() } label: {
                    Text(direct ? "Back to conversation" : "Back to channel")
                        #if os(iOS)
                        .frame(minHeight: 44)
                        #else
                        .frame(minHeight: 28)
                        #endif
                        .contentShape(Rectangle())
                }.buttonStyle(.plain).font(CaperTheme.font(12)).modifier(ControlHover())
            }.padding(.horizontal, 18).frame(height: 50)
                .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
            ScrollViewReader { proxy in
                ScrollView(showsIndicators: false) {
                    let replyRows = entries(replies)
                    let root = chat.displayedMessages.first(where: { $0.id == chat.threadRootID })
                    // The root's reactions grow the thread as well as its replies'.
                    let recentRows: [TimelineEntry] = (root.map { entries([$0]) } ?? []) + replyRows.suffix(TimelinePin.recentRows)
                    LazyVStack(spacing: 0) {
                        let threadPending: PendingMessage? = chat.pendingMessage.flatMap { $0.threadRootId == chat.threadRootID ? $0 : nil }
                        let grouping = MessageGrouping.layout(replyRows, inThread: true, pending: threadPending, author: chat.currentAuthor)
                        // Before any row is built, so rows see this render's arrivals. The root is never one.
                        let _ = arrivals.observe("thread", key: chat.threadRootID ?? "",
                                                 ids: chat.threadMessages.map(\.clientMessageId) + (threadPending.map { [$0.id] } ?? []),
                                                 live: chat.threadRootID != nil && !chat.threadLoading && !chat.threadHasNewer)
                        if let root {
                            threadEntries(entries([root]))
                            if let summary = root.thread, summary.replyCount > 0 {
                                Text("\(summary.replyCount) \(summary.replyCount == 1 ? "reply" : "replies")").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).padding(12)
                            }
                        }
                        if chat.threadLoading && replies.isEmpty {
                            MessageSkeleton(rows: 2, label: "Loading thread replies")
                        }
                        // Styled like the channel's "Load older messages" row.
                        if let error = chat.threadError {
                            HStack(spacing: 6) {
                                Text(error).foregroundStyle(CaperTheme.error).fixedSize(horizontal: false, vertical: true)
                                Button("Retry") { Task { await chat.loadThread() } }.disabled(chat.threadLoading)
                            }.font(CaperTheme.font(11, weight: .medium)).padding(.horizontal, 18).frame(minHeight: 44)
                        }
                        if chat.threadHasMore {
                            Button(chat.threadLoading ? "Loading…" : "Load older replies", action: loadOlderReplies)
                                .disabled(chat.threadLoading).frame(height: 44)
                        }
                        threadEntries(replyRows, grouping: grouping)
                        if chat.threadHasNewer {
                            Button(chat.threadLoading ? "Loading…" : "Load newer replies") { Task { await chat.loadThread(newer: true) } }
                                .disabled(chat.threadLoading).frame(height: 44)
                        }
                        if replies.isEmpty && !chat.threadLoading && chat.threadError == nil { Text(chat.isPreview ? "No replies yet." : "No replies yet. Start the thread.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted).padding(18) }
                        if let pending = threadPending {
                            PendingMessageRow(pending: pending, author: chat.currentAuthor, error: chat.error, rejected: chat.sendRejected, canEdit: chat.threadDraft.isEmpty && chat.threadAttachmentDrafts.isEmpty, chat: chat,
                                uploadFailed: chat.sendUploadFailed, grouped: grouping.pendingGrouped,
                                retry: { Task { await chat.send(inThread: true) } },
                                edit: { _ = chat.discardRejected(edit: true) }, dismiss: { _ = chat.discardRejected() })
                                .modifier(Entrance.message(arrivals.claim(pending.id)))
                        }
                    }
                    // Follow reaction height changes as well as a shorter viewport.
                    .onGeometryChange(for: TimelinePin.self) { TimelinePin($0, viewport: MentionCard.timelineSpace, firstID: replies.first?.id, recent: recentRows) } action: { pin in
                        if timelinePin.needsReveal(to: pin), !chat.threadHasNewer, chat.focusedMessageID == nil, let id = replies.last?.id ?? chat.threadRootID { proxy.scrollTo(id, anchor: .bottom) }
                    }
                }
                #if os(iOS)
                .scrollDismissesKeyboard(.immediately)
                #endif
                #if os(macOS)
                .accessibilityIdentifier("thread-timeline")
                #endif
                .coordinateSpace(.named(MentionCard.timelineSpace))
                .modifier(MentionCardHost(target: $mentionCard, model: model, viewerID: viewerID))
                .onChange(of: replies.last?.id) { old, id in
                    guard chat.focusedMessageID == nil, let id else { return }
                    // As on web: another person's reply follows only a reader at the end;
                    // a newly opened thread and your own reply always show the latest.
                    let replaced = old.map { old in !replies.contains { $0.id == old } } ?? true
                    if replaced || timelinePin.atBottom || replies.last?.clientMessageId == localReplyID { proxy.scrollTo(id, anchor: .bottom) }
                }
                .onChange(of: chat.pendingMessage?.id) { _, id in
                    if let id, let root = chat.pendingMessage?.threadRootId, root == chat.threadRootID { localReplyID = id }
                }
                // Older replies arrived above: the reply that was first stays at the top.
                .onChange(of: chat.threadMessages.first?.id) { _, _ in
                    guard let anchor = olderAnchor else { return }
                    olderAnchor = nil
                    proxy.scrollTo(anchor, anchor: .top)
                }
                    .onChange(of: chat.focusRevision, initial: true) { _, _ in
                        if let id = chat.focusedMessageID, replies.contains(where: { $0.id == id }) {
                            revealedBlocked.insert(id)
                            Task { @MainActor in
                                await Task.yield()
                                if chat.focusedMessageID == id { proxy.scrollTo(id, anchor: .center) }
                            }
                        }
                    }
            }
            if chat.isPreview { Text(direct ? "You can’t reply in this conversation." : "Join the channel to reply.").font(CaperTheme.font(12)).padding(18) }
            else { VStack(alignment: .leading, spacing: 8) {
                if let root = chat.threadRootID, let reference = chat.replyTargets[root] { NativeReplyDraft(reference: reference, chat: chat, root: root) }
                if let error = chat.error, chat.pendingMessage == nil { Text(error).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error) }
                if let error = chat.sessionError {
                    // As under the channel's composer.
                    HStack(spacing: 8) {
                        Text(error).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
                        Button("Retry session") { Task { await chat.retrySession() } }.buttonStyle(.plain)
                            .font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.text).modifier(ControlHover())
                    }
                }
                if let notice = chat.threadAttachmentNotice {
                    Text(notice).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.terracottaBright)
                }
                AttachmentDraftsView(chat: chat, inThread: true)
                #if os(iOS)
                ComposerSuggestionsView(controller: composerAutocomplete)
                #endif
                HStack(alignment: .bottom, spacing: 0) {
                    AttachmentPickerButton(chat: chat, inThread: true).padding(.leading, 4)
                    // Like the channel composer: the placeholder sits behind the clear editor, which draws none.
                    ZStack(alignment: .topLeading) {
                        if chat.threadDraft.isEmpty {
                            Text("Reply to thread…").font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted)
                                .padding(.leading, 11).padding(.top, 12).allowsHitTesting(false).accessibilityHidden(true)
                        }
                        threadComposer
                    }
                        .frame(minHeight: 72, maxHeight: 174)
                        .fixedSize(horizontal: false, vertical: true)
                        #if os(iOS)
                        .overlay(alignment: .topLeading) { ParityKeyboardProbe() }
                        #endif
                    ComposerEmojiButton(controller: composerAutocomplete, enabled: !chat.threadLoading).padding(.trailing, 2)
                }
                    // Clipped too: the fill's square corners showed outside the rounded border.
                    .background(CaperTheme.composer).clipShape(RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                    #if os(macOS)
                    .anchorPreference(key: ThreadComposerAnchorKey.self, value: .bounds) { $0 }
                    #endif
                HStack {
                    Toggle(direct ? "Also send to conversation" : "Also send to #\(chat.channelName)", isOn: $chat.threadBroadcast).font(CaperTheme.font(11)).disabled(chat.pendingMessage != nil)
                    sendReplyButton
                }
                if let pending = chat.pendingMessage, pending.threadRootId != chat.threadRootID,
                   !chat.sending, chat.error != nil || chat.sendRejected {
                    Text("Confirm or dismiss the pending message before sending a reply.").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
                }
            }.padding(.horizontal, 12)
            .padding(.top, 12)
            .padding(.bottom, 16)
            }
        }.background { CaperTheme.conversation.ignoresSafeArea() }
            .overlay(alignment: .leading) { Rectangle().fill(CaperTheme.border).frame(width: 1) }
            .dropDestination(for: DroppedAttachment.self) { items, _ in
                guard let target = fileDropTarget ?? chat.attachmentTarget(inThread: true), !items.isEmpty else {
                    items.forEach { AttachmentStaging.remove($0.file.url) }
                    return false
                }
                chat.addAttachments(items.map(\.file), to: target)
                fileDropTarget = nil
                return true
            } isTargeted: {
                fileDropTargeted = $0
                if $0 { fileDropTarget = chat.attachmentTarget(inThread: true) }
            }
            .overlay {
                if fileDropTargeted && chat.canAttach {
                    let text = AttachmentPolicy.dropText(channel: chat.channelName, direct: direct, attached: chat.threadAttachmentDrafts.count)
                    FileDropOverlay(title: "Add files to thread", detail: text.detail)
                }
            }
            #if os(macOS)
            .modifier(ComposerSuggestionsOverlay<ThreadComposerAnchorKey>(controller: composerAutocomplete))
            #endif
            .sheet(item: $reactionMessage) { message in
                #if os(iOS)
                MessageActionsSheet(message: message, chat: chat, showingEmojiPicker: $showingEmojiPicker,
                    canReact: chat.canInteract,
                    canPin: chat.canInteract && !chat.pendingPins.contains(message.id),
                    togglePin: { reactionMessage = nil; Task { await chat.setPin(messageID: message.id, active: message.pin == nil) } },
                    reply: nil, inThread: true,
                    // The conversation's forward sheet is under this cover on iPhone; the thread presents its own.
                    canForward: chat.canForward, forward: { forwardMessage = message },
                    quickReaction: { emoji in
                        let own = message.reactions?.first { $0.emoji == emoji }?.authorIds.contains(chat.currentAuthor?.id ?? "") == true
                        reactionMessage = nil; Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: !own) }
                    }, selectReaction: { emoji in reactionMessage = nil; Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: true) } },
                    block: BlockedMessages.canBlock(message.author, viewerID: chat.viewerID, blocked: chat.blockedAuthorIDs)
                        ? { blockTarget = BlockTarget(author: message.author) } : nil,
                    unblock: BlockedMessages.canUnblock(message.author, viewerID: chat.viewerID, blocked: chat.blockedAuthorIDs)
                        ? { chat.unblock(message.author) } : nil,
                    viewReactions: { emoji in reactors.show(message.id, emoji) })
                #else
                ReactionPicker { emoji in reactionMessage = nil; Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: true) } }
                #endif
            }
            .sheet(item: $reactorsTarget) { target in ReactorsSheet(chat: chat, messageID: target.messageID, emoji: target.emoji, viewerID: chat.currentAuthor?.id) }
            #if os(iOS)
            .sheet(item: $forwardMessage) { message in ForwardPickerView(chat: chat, message: message) }
            #endif
            .onChange(of: reactionMessage?.id) { _, _ in showingEmojiPicker = false }
            .onChange(of: chat.editingContext) { _, _ in reactionMessage = nil; reactorsTarget = nil; forwardMessage = nil }
            .modifier(BlockConfirmation(target: $blockTarget) { await chat.block($0) })
            #if os(iOS)
            // The thread is a cover on iPhone, which only it can present over.
            .modifier(MediaViewerHost())
            #endif
            // One element for the whole panel; without it SwiftUI copies the
            // identifier onto every child and a query for the panel is ambiguous.
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("message-thread")
    }

    private var threadComposer: NativeMessageComposer {
        let paste: (() -> Bool)? = chat.canAttach ? { chat.pasteAttachments(inThread: true) } : nil
        return NativeMessageComposer(text: $chat.threadDraft, placeholder: "Reply to thread…", controller: composerAutocomplete,
                                     mentions: mentions, submit: { Task { await chat.send(inThread: true) } }, pasteAttachments: paste)
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
@MainActor private func voiceAvailabilityHelp(_ available: Bool?) -> String? {
    switch available {
    case true?: return nil
    case false?: return "Joining is not available at this time."
    case nil: return "Checking voice availability…"
    }
}

private struct PrimaryIconButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.modifier(PrimaryActionFill(pressed: configuration.isPressed))
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

    static func date(_ value: String) -> Date? { ChatTimestamp.date(value) }

    static func sameLocalDay(_ first: String, _ second: String, calendar: Calendar = .current) -> Bool {
        guard let firstDate = date(first), let secondDate = date(second) else { return false }
        return calendar.isDate(firstDate, inSameDayAs: secondDate)
    }

    static func fullDateLabel(_ value: String, locale: Locale = .current, timeZone: TimeZone = .current) -> String? {
        guard let date = date(value) else { return nil }
        return ChatTimestamp.fullDate(date, locale: locale, timeZone: timeZone)
    }
}

/// Shared timestamp parsers and the divider's date formatter. Timelines parse
/// every row on each render (grouping, dividers, times), and creating
/// ISO8601DateFormatter/DateFormatter per call is expensive.
enum ChatTimestamp {
    private static let fractional: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()
    private static let whole = ISO8601DateFormatter()
    private static let lock = NSLock()
    /// The last full-date formatter, rebuilt when the locale or time zone changes.
    private static var fullDateFormatter: (key: String, formatter: DateFormatter)?

    /// RFC 3339, with or without fractional seconds.
    static func date(_ value: String) -> Date? {
        fractional.date(from: value) ?? whole.date(from: value)
    }

    /// "Monday, September 28, 2026" (the locale's full date style).
    static func fullDate(_ date: Date, locale: Locale, timeZone: TimeZone) -> String {
        let key = "\(locale.identifier)|\(timeZone.identifier)"
        lock.lock(); defer { lock.unlock() }
        if let cached = fullDateFormatter, cached.key == key { return cached.formatter.string(from: date) }
        let formatter = DateFormatter()
        formatter.locale = locale
        formatter.timeZone = timeZone
        formatter.dateStyle = .full
        formatter.timeStyle = .none
        fullDateFormatter = (key, formatter)
        return formatter.string(from: date)
    }
}

/// Web's quick reactions, first in message actions.
private enum MessageQuickReactions {
    static let emoji = ["👍", "❤️", "😂", "🎉", "👀"]
}

private struct MessageRow: View {
    let message: ChatMessage
    @Bindable var chat: ChatModel
    let currentUserID: String?
    let reactors: ReactorContext
    let mentionCards: MentionCardContext
    var inThread = false
    var inPins = false
    /// Compact, under the same person's previous row (see `MessageGrouping`):
    /// no avatar or header, presentation only.
    var grouped = false
    /// A grouped row follows: this row gives up its bottom padding.
    var continued = false
    /// Asks the containing view to confirm blocking this message's author.
    var requestBlock: ((ChatAuthor) -> Void)? = nil
    var forwardOpened: (() -> Void)? = nil
    let showReactionPicker: () -> Void
    private var canBlockAuthor: Bool {
        requestBlock != nil && BlockedMessages.canBlock(message.author, viewerID: chat.viewerID, blocked: chat.blockedAuthorIDs)
    }
    private var edited: Bool { message.forward == nil && (message.revision ?? 1) > 1 }
    /// A shown message from someone you blocked offers Unblock instead.
    private var canUnblockAuthor: Bool {
        requestBlock != nil && BlockedMessages.canUnblock(message.author, viewerID: chat.viewerID, blocked: chat.blockedAuthorIDs)
    }
    @State private var editing = false
    @State private var history = false
    @State private var pressing = false
    #if os(macOS)
    /// Room the first line leaves for the hover actions: 24pt each, 2pt apart
    /// (Reply in thread, Reply, Add reaction and ⋯), plus a gap.
    private var actionsInset: CGFloat { inThread ? 82 : 108 }
    @State private var controlsHovered = false
    @State private var actionsTop: CGFloat = 6
    @FocusState private var replyFocused: Bool
    @FocusState private var replyMessageFocused: Bool
    @FocusState private var reactionFocused: Bool
    @FocusState private var actionsFocused: Bool
    @FocusState private var threadLinkFocused: Bool
    @FocusState private var editedFocused: Bool
    #endif
    var body: some View {
        let mentionsMe = MentionAutocomplete.mentionsCurrentUser(message, currentUserID: currentUserID)
        let pills = MentionAutocomplete.pills(in: message.content.text, mentions: message.content.mentions)
        let rendered = messageText
        let row = VStack(alignment: .leading, spacing: 5) {
            if inPins, let pin = message.pin {
                Button { mentionCards.openAuthor?(pin.author) } label: {
                    HStack(spacing: 4) {
                        CaperIcon(name: "pin", size: 12)
                        Text("Pinned by \(pin.author.name)")
                    }.contentShape(Rectangle())
                }
                    .buttonStyle(.plain).font(CaperTheme.font(10, weight: .medium)).foregroundStyle(CaperTheme.muted)
                    .accessibilityLabel("Open profile for \(pin.author.name)")
                    .padding(.leading, 44)
                    #if os(macOS)
                    .padding(.trailing, actionsInset)
                    .onContinuousHover(coordinateSpace: CoordinateSpace.named(MentionCard.timelineSpace)) { phase in
                        if case .active(let point) = phase {
                            mentionCards.pointer.location = point
                            mentionCards.openAuthor?(pin.author)
                        }
                    }
                    #else
                    .highPriorityGesture(LongPressGesture(minimumDuration: 0.5).onEnded { _ in mentionCards.openAuthor?(pin.author) })
                    #endif
                    .accessibilityIdentifier("pinned-by-\(message.id)")
            }
            if let reference = message.replyTo { NativeReplyContext(reference: reference, motionKey: message.clientMessageId, chat: chat) }
            // A grouped row lines its text up with the avatar column's time.
            HStack(alignment: grouped ? .firstTextBaseline : .top, spacing: 10) {
                if grouped { groupedGutter } else {
                    authorProfile { Avatar(name: message.author.name, size: 34, avatarID: message.author.avatarId) }
                }
                VStack(alignment: .leading, spacing: 4) {
                if !grouped {
                    HStack(alignment: .firstTextBaseline, spacing: 7) {
                        authorProfile { Text(message.author.name).font(CaperTheme.font(13, weight: .bold)) }
                        if message.author.isAdmin { AdminBadge() }
                        if message.author.isGuest { Text("GUEST").font(CaperTheme.font(9, weight: .bold)).foregroundStyle(CaperTheme.muted).padding(.horizontal, 5).overlay(RoundedRectangle(cornerRadius: 4).stroke(CaperTheme.border)) }
                        Text(inPins ? ChatDateDivider.date(message.createdAt)?.formatted(date: .abbreviated, time: .shortened) ?? timeLabel(message.createdAt) : timeLabel(message.createdAt)).font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
                        if edited { editedMarker }
                    }
                    #if os(macOS)
                    .padding(.trailing, actionsInset)
                    #endif
                }
                // File-only messages carry empty text; hide the empty row.
                if !message.content.text.isEmpty || (grouped && edited) {
                HStack(alignment: .lastTextBaseline, spacing: 7) {
                    rendered.text.font(CaperTheme.font(14)).foregroundStyle(Color(red: 222/255, green: 223/255, blue: 224/255))
                        // Links take the tint. Person pills are links too: keep them in
                        // the pill text colour, not the accent, when the text has any.
                        .tint(pills.isEmpty ? CaperTheme.terracottaBright : CaperTheme.text)
                        .modifier(SwipeGuardedLinks())
                        .modifier(MentionPillInteraction(pills: pills, hasLinks: rendered.hasLinks, cards: mentionCards))
                    if grouped && edited { editedMarker }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                #if os(macOS)
                // Without a header, the first line sits beside the hover actions.
                .padding(.trailing, grouped ? actionsInset : 0)
                #endif
                }
                if let attachments = message.content.attachments, !attachments.isEmpty {
                    MessageAttachmentsView(attachments: attachments, chat: chat)
                }
                if MentionAutocomplete.asksFeedback(message) {
                    Text(FeedbackInbox.notified).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                        .accessibilityIdentifier("feedback-notified-\(message.id)")
                }
                ForwardCardView(message: message) {
                    guard !SwipeTapGuard.swiping else { return }
                    Task { if await chat.onOpenForward?(message) == true { forwardOpened?() } }
                }
                ReactionRow(message: message, chat: chat, reactors: reactors)
                if let error = chat.pinErrors[message.id] {
                    HStack(spacing: 8) {
                        Text(error)
                        Button("Retry") { if !SwipeTapGuard.swiping { Task { await chat.retryPin(messageID: message.id) } } }
                            .disabled(!chat.canInteract || chat.pendingPins.contains(message.id))
                        // As on web, the error can be put away without retrying.
                        Button("Dismiss") { if !SwipeTapGuard.swiping { chat.pinErrors[message.id] = nil } }
                    }.font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.error)
                }
                if !inThread && (message.threadRootId != nil || (message.thread?.replyCount ?? 0) > 0) {
                    Button { if !SwipeTapGuard.swiping { Task { await chat.openThread(message.threadRootId ?? message.id) } } } label: {
                        HStack(spacing: 5) {
                            if message.threadRootId == nil, let summary = message.thread {
                                ForEach(summary.participants, id: \.id) { Avatar(name: $0.name, size: 24, avatarID: $0.avatarId) }
                                Text("\(summary.replyCount) \(summary.replyCount == 1 ? "reply" : "replies") · View thread")
                            } else { CaperIcon(name: "message-square", size: 14); Text("Replied to a thread · View thread") }
                        }.font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.terracottaBright)
                    }.buttonStyle(.plain).frame(minHeight: 32)
                        #if os(macOS)
                        .focused($threadLinkFocused).modifier(ControlHover(isFocused: threadLinkFocused))
                        #else
                        .modifier(ControlHover())
                        #endif
                        .accessibilityLabel("View thread")
                }
                if let error = chat.reactionErrors[message.id] {
                    HStack(spacing: 8) {
                        Text(error)
                        Button("Retry") { if !SwipeTapGuard.swiping { Task { await chat.retryReaction(messageID: message.id) } } }
                            .disabled(!chat.canInteract)
                        Button("Dismiss") { if !SwipeTapGuard.swiping { chat.reactionErrors[message.id] = nil } }
                    }.font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.error)
                }
            }
            }
            #if os(macOS)
            .onGeometryChange(for: CGFloat.self) { $0.frame(in: .named("reply-row-\(message.id)")).minY } action: { if message.replyTo != nil { actionsTop = max(0, $0 - 4) } }
            #endif
        // As on web, a run reads as one block: 2pt instead of 10 between its rows.
        }.padding(.horizontal, 18).padding(.top, grouped ? 2 : 10).padding(.bottom, continued ? 2 : 10)
            .coordinateSpace(name: "reply-row-\(message.id)")
            .background(CaperTheme.text.opacity(pressing ? 0.08 : 0))
            .background(rowBackground(mentionsMe: mentionsMe))
            .overlay(alignment: .leading) {
                if showsAccentBar(mentionsMe: mentionsMe) { Rectangle().fill(CaperTheme.terracotta).frame(width: 3).accessibilityHidden(true) }
            }
            // An identifier on a plain container is copied onto every child,
            // replacing their own (add-reaction-…, reaction chips). Make the
            // row a containing element so children keep their identifiers.
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("message-row-\(message.id)")
            .sheet(isPresented: $editing) { MessageEditorView(chat: chat, message: message) { editing = false } }
            .sheet(isPresented: $history) { MessageHistoryView(chat: chat, message: message) { history = false } }
        #if os(iOS)
        row.contentShape(Rectangle())
            .onLongPressGesture(perform: showReactionPicker, onPressingChanged: { pressing = $0 })
            .onDisappear { pressing = false }
            .accessibilityAction(named: Text("Message actions")) { showReactionPicker() }
        #else
        row
            .overlay(alignment: .topTrailing) {
                HStack(spacing: 2) {
                    if !inThread {
                        Button { Task { await chat.openThread(message.threadRootId ?? message.id) } } label: {
                            CaperIcon(name: "message-square", size: 14).frame(width: 24, height: 24)
                        }.buttonStyle(.plain).focused($replyFocused).help("Reply in thread").accessibilityLabel("Reply in thread")
                            .modifier(ControlHover(isFocused: replyFocused))
                    }
                    Button { chat.reply(to: message, inThread: inThread && !inPins) } label: {
                        CaperIcon(name: "reply", size: 14).frame(width: 24, height: 24)
                    }.buttonStyle(.plain).focused($replyMessageFocused).help("Reply").accessibilityLabel("Reply")
                        .modifier(ControlHover(isFocused: replyMessageFocused))
                        .disabled(!chat.canInteract || chat.pendingMessage != nil)
                    Button(action: showReactionPicker) {
                        CaperIcon(name: "smile-plus", size: 14).frame(width: 24, height: 24)
                            .contentShape(Rectangle())
                    }.buttonStyle(.plain).frame(width: 24, height: 24)
                        .focused($reactionFocused).help("Add reaction").accessibilityLabel("Add reaction")
                        .modifier(ControlHover(isFocused: reactionFocused))
                        .disabled(!chat.canInteract)
                        .accessibilityIdentifier("add-reaction-\(message.id)")
                    Menu {
                        messageMenuItems
                    } label: {
                        // Lucide's ellipsis, as on the sidebar rows and web's More actions.
                        CaperIcon(name: "ellipsis", size: 14).frame(width: 24, height: 24)
                    }.menuStyle(.borderlessButton).menuIndicator(.hidden)
                        // Native Menu otherwise expands the overlay to the row's
                        // width, moving Add reaction away from the trailing edge.
                        .frame(width: 24, height: 24).focused($actionsFocused)
                        .modifier(ControlHover(isFocused: actionsFocused))
                        .help("More actions")
                        .accessibilityLabel("Message options")
                }.opacity(controlsHovered || replyFocused || replyMessageFocused || reactionFocused || actionsFocused ? 1 : 0)
                    .allowsHitTesting(controlsHovered || replyFocused || replyMessageFocused || reactionFocused || actionsFocused)
                    .padding(.trailing, 18).padding(.top, message.replyTo == nil ? 6 : actionsTop)
            }
            // Track the whole row, including its overlay. Entering a message
            // action must not hide that action before the pointer can click it.
            .contentShape(Rectangle())
            .onHover { controlsHovered = $0 }
            .contextMenu { messageMenuItems }
        #endif
    }
    @ViewBuilder private func authorProfile<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        if !message.author.isGuest, let openAuthor = mentionCards.openAuthor {
            Button { openAuthor(message.author) } label: { content() }
                .buttonStyle(ProfileButtonStyle())
                .accessibilityLabel("Open profile for \(message.author.name)")
                #if os(macOS)
                .onContinuousHover(coordinateSpace: CoordinateSpace.named(MentionCard.timelineSpace)) { phase in
                    switch phase {
                    case .active(let point): mentionCards.pointer.location = point
                    case .ended: mentionCards.pointer.location = nil
                    }
                }
                #endif
        } else { content() }
    }
    #if os(macOS)
    /// Web's message actions, in the ⋯ menu and the context menu. A quick
    /// reaction is checked when it is yours; choosing it again removes it.
    @ViewBuilder private var messageMenuItems: some View {
        ForEach(MessageQuickReactions.emoji, id: \.self) { emoji in
            Toggle("React with \(emoji)", isOn: Binding(get: { reacted(emoji) }, set: { react(emoji, active: $0) }))
                .disabled(!chat.canInteract)
        }
        // The full picker, as from the hover row's Add reaction.
        Button("Add reaction…", action: showReactionPicker).disabled(!chat.canInteract)
        Divider()
        Button("Reply") { chat.reply(to: message, inThread: inThread && !inPins) }.disabled(!chat.canInteract || chat.pendingMessage != nil)
        if !inThread { Button("Reply in thread") { Task { await chat.openThread(message.threadRootId ?? message.id) } } }
        if chat.canForward { Button("Forward message") { chat.forwardTarget = message } }
        if chat.canEdit(message) { Button("Edit message") { editing = true } }
        if message.forward == nil && (message.revision ?? 1) > 1 { Button("Message history") { history = true } }
        if let reaction = message.reactions?.first { Button("View reactions") { reactors.show(message.id, reaction.emoji) } }
        Button(message.pin == nil ? "Pin message" : "Unpin message") {
            Task { await chat.setPin(messageID: message.id, active: message.pin == nil) }
        }.disabled(!chat.canInteract || chat.pendingPins.contains(message.id))
        // Attachment-only messages have no text to copy.
        if !message.content.text.isEmpty { Button("Copy text") { copy(message.content.text) } }
        Button("Copy message ID") { copy(message.id) }
        if canBlockAuthor {
            Divider()
            Button("Block \(message.author.name)…", role: .destructive) { requestBlock?(message.author) }
        } else if canUnblockAuthor {
            Divider()
            Button("Unblock \(message.author.name)") { chat.unblock(message.author) }
        }
    }
    private func reacted(_ emoji: String) -> Bool {
        guard let id = chat.currentAuthor?.id else { return false }
        return message.reactions?.first(where: { $0.emoji == emoji })?.authorIds.contains(id) == true
    }
    private func react(_ emoji: String, active: Bool) {
        Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: active) }
    }
    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
    #endif
    /// The jump target (a link or search result) and the open thread's root
    /// are selections: terracotta, like web. Gold stays reserved for pins.
    private var jumpTarget: Bool { !inPins && chat.focusedMessageID == message.id }
    private var threadOpen: Bool { !inThread && chat.threadRootID == message.id }
    /// Web's inset 3px terracotta bar on selected rows and mentions of you.
    private func showsAccentBar(mentionsMe: Bool) -> Bool { jumpTarget || threadOpen || mentionsMe }
    private func rowBackground(mentionsMe: Bool) -> Color {
        if jumpTarget || threadOpen { return CaperTheme.terracotta.opacity(0.12) }
        if mentionsMe { return CaperTheme.terracotta.opacity(0.08) }
        if message.replyTo != nil { return CaperTheme.text.opacity(0.03) }
        return inPins && message.pin != nil ? CaperTheme.pinGold.opacity(0.06) : .clear
    }
    /// Resolved `@mention` tokens render as pills: #F3F4F5 medium text on 24%
    /// terracotta. SwiftUI `Text` styles an inline run's background but cannot
    /// pad or round it, so the pill is a square-cornered span tight to its
    /// glyphs (spec: 2pt padding, 4pt corners). Unresolved names stay plain.
    /// Links are found after mentions, in the text between pills only.
    private var messageText: (text: Text, hasLinks: Bool) {
        let content = message.content
        let segments = MentionAutocomplete.segments(in: content.text, mentions: content.mentions)
        var attributed = AttributedString()
        var styled = false
        var hasLinks = false
        for segment in segments {
            guard segment.highlighted else {
                if let linked = MessageLinks.attributed(segment.text) {
                    attributed.append(linked)
                    hasLinks = true
                } else {
                    attributed.append(AttributedString(segment.text))
                }
                continue
            }
            var part = AttributedString(segment.text)
            part.font = CaperTheme.font(14, weight: .medium)
            part.foregroundColor = CaperTheme.text
            part.backgroundColor = CaperTheme.terracotta.opacity(0.24)
            // A person's pill opens the mention card; @everyone/@here stay inert.
            if let user = segment.user { part.link = MentionCard.url(for: user) }
            attributed.append(part)
            styled = true
        }
        guard styled || hasLinks else { return (Text(content.text), false) }
        return (Text(attributed), hasLinks)
    }
    /// "(edited)": opens the edit history, in the header or, on a grouped row, after the text.
    private var editedMarker: some View {
        let marker = Button("(edited)") { if !SwipeTapGuard.swiping { history = true } }.buttonStyle(.plain).font(CaperTheme.font(10)).foregroundStyle(CaperTheme.muted)
        #if os(macOS)
        // Its own focus: the timeline is focusable, and an unbound focus ring
        // would follow the timeline's focus instead of this control's.
        return marker.focused($editedFocused).modifier(ControlHover(isFocused: editedFocused))
            .accessibilityLabel("Message history, version \(message.revision ?? 1)")
        #else
        return marker.modifier(ControlHover()).accessibilityLabel("Message history, version \(message.revision ?? 1)")
        #endif
    }
    /// A grouped row's empty avatar column. With a pointer (macOS) it shows
    /// the short time ("8:28") while the row is hovered or one of its controls
    /// has keyboard focus; on touch it stays empty. VoiceOver always gets the
    /// author and time from it, as from the header it replaces.
    private var groupedGutter: some View {
        #if os(macOS)
        let visible = controlsHovered || replyFocused || reactionFocused || actionsFocused || threadLinkFocused
        #else
        let visible = false
        #endif
        let time = ChatDateDivider.date(message.createdAt)?.formatted(.dateTime.hour(.defaultDigits(amPM: .omitted)).minute()) ?? ""
        // A clear colour, not opacity or `hidden()`, so VoiceOver keeps the element.
        return Text(time).font(CaperTheme.font(10)).monospacedDigit().lineLimit(1).fixedSize()
            .foregroundStyle(visible ? CaperTheme.muted : Color.clear)
            .frame(width: 34, alignment: .trailing)
            .accessibilityLabel("\(message.author.name), \(timeLabel(message.createdAt))")
    }
    private func timeLabel(_ value: String) -> String {
        guard let date = ChatDateDivider.date(value) else { return "" }
        return date.formatted(date: .omitted, time: .shortened)
    }
}

extension MessageLinks {
    /// `text` with its links in terracotta-bright, underlined, and opening in
    /// the system browser (the default `openURL`); nil when it has none. The
    /// visible text is never shortened. Hosts set `.tint` to the same colour:
    /// SwiftUI may draw link runs in the tint rather than their own colour.
    static func attributed(_ text: String) -> AttributedString? {
        let runs = segments(in: text)
        guard runs.contains(where: { $0.href != nil }) else { return nil }
        var attributed = AttributedString()
        for run in runs {
            var part = AttributedString(run.text)
            // iOS 17/macOS 14 URL parsing percent-encodes what a URL can't hold, e.g. a non-ASCII path.
            if let href = run.href, let url = URL(string: href) {
                part.link = url
                part.foregroundColor = CaperTheme.terracottaBright
                part.underlineStyle = Text.LineStyle(pattern: .solid, color: CaperTheme.terracottaBright)
            }
            attributed.append(part)
        }
        return attributed
    }
}

/// How a message's person pills reach the mention card.
private struct MentionCardContext {
    let pointer: MentionPointer
    /// "Open profile for <display name or @username>".
    let name: (MentionPill) -> String
    let open: (MentionPill) -> Void
    var openAuthor: ((ChatAuthor) -> Void)? = nil
}

/// The pointer over message text, in the timeline's space, read only when a
/// pill opens. A plain reference box, so hover tracking never re-renders rows.
private final class MentionPointer {
    var location: CGPoint?
}

private struct MentionCardTarget: Identifiable {
    enum Reference {
        case mention(MentionPill)
        case author(ChatAuthor)
    }
    let id = UUID()
    let reference: Reference
    /// macOS: the click point; nil (keyboard or VoiceOver) centres the popover.
    let anchor: CGRect?

    /// The click point becomes a pill-high rect, so the popover sits below the pill.
    init(pill: MentionPill, pointer: CGPoint?) {
        self.init(reference: .mention(pill), pointer: pointer)
    }

    init(author: ChatAuthor, pointer: CGPoint?) {
        self.init(reference: .author(author), pointer: pointer)
    }

    private init(reference: Reference, pointer: CGPoint?) {
        self.reference = reference
        anchor = pointer.map { CGRect(x: $0.x - 1, y: $0.y - 9, width: 2, height: 18) }
    }
}

private extension AppModel {
    /// The mention card's data, from what this client has already loaded.
    func mentionPerson(_ pill: MentionPill, viewerID: String?) -> MentionCardPerson {
        MentionCard.resolve(pill, members: detail?.members ?? [], people: people,
                            peers: directMessages.map(\.peer), account: account, viewerID: viewerID)
    }

    func mentionPerson(_ target: MentionCardTarget, viewerID: String?) -> MentionCardPerson {
        switch target.reference {
        case .mention(let pill): mentionPerson(pill, viewerID: viewerID)
        case .author(let author): MentionCard.resolve(author, members: detail?.members ?? [], people: people,
            peers: directMessages.map(\.peer), account: account, viewerID: viewerID)
        }
    }
}

/// Presents the mention card: a popover at the clicked pill on macOS, a sheet
/// on iPhone. Attach it to the scroll view that names `MentionCard.timelineSpace`,
/// so the pointer and the anchor share coordinates (the channel and the thread
/// panel each host their own).
private struct MentionCardHost: ViewModifier {
    @Binding var target: MentionCardTarget?
    let model: AppModel
    let viewerID: String?
    /// Block closes the card first, as on web, so the confirmation belongs to the host.
    @State private var blockTarget: BlockTarget?

    @ViewBuilder func body(content: Content) -> some View {
        #if os(macOS)
        content.popover(item: protectedTarget, attachmentAnchor: anchor, arrowEdge: .bottom) { target in
            MentionCardView(model: model, person: model.mentionPerson(target, viewerID: viewerID),
                            requestBlock: { blockTarget = $0 }) { self.target = nil }
        }
        .modifier(BlockConfirmation(target: $blockTarget) { await model.chat.block($0) })
        #else
        content.sheet(item: protectedTarget) { target in
            MentionCardView(model: model, person: model.mentionPerson(target, viewerID: viewerID),
                            requestBlock: { blockTarget = $0 }) { self.target = nil }
        }
        .modifier(BlockConfirmation(target: $blockTarget) { await model.chat.block($0) })
        #endif
    }

    private var protectedTarget: Binding<MentionCardTarget?> {
        Binding(get: { target }, set: { next in
            if let target {
                let person = model.mentionPerson(target, viewerID: viewerID)
                let draft = model.profileMessageDraft(personID: person.id)
                if draft.preventsImplicitDismissal { return }
            }
            target = next
        })
    }

    #if os(macOS)
    private var anchor: PopoverAttachmentAnchor {
        guard let rect = target?.anchor else { return .point(.center) }
        return .rect(.rect(rect))
    }
    #endif
}

#if os(iOS)
/// Observe only the sheet's backdrop, keeping native detents, dragging and keyboard handling.
private struct ProfileSheetOutsideTap: UIViewControllerRepresentable {
    let close: () -> Void

    func makeUIViewController(context: Context) -> Controller { Controller() }
    func updateUIViewController(_ controller: Controller, context: Context) { controller.close = close }
    static func dismantleUIViewController(_ controller: Controller, coordinator: ()) { controller.removeTap() }

    final class Controller: UIViewController, UIGestureRecognizerDelegate {
        var close: (() -> Void)?
        private weak var presentation: UIPresentationController?
        private var tap: UITapGestureRecognizer?

        override func loadView() {
            view = UIView()
            view.isUserInteractionEnabled = false
        }

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            var ancestor = parent
            while let controller = ancestor {
                if let presentation = controller.presentationController, let container = presentation.containerView {
                    removeTap()
                    self.presentation = presentation
                    let tap = UITapGestureRecognizer(target: self, action: #selector(outsideTap))
                    tap.delegate = self
                    tap.cancelsTouchesInView = false
                    container.addGestureRecognizer(tap)
                    self.tap = tap
                    break
                }
                ancestor = controller.parent
            }
        }

        func removeTap() {
            if let tap { tap.view?.removeGestureRecognizer(tap) }
            tap = nil
        }

        func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
            guard let presentation, let container = presentation.containerView, let sheet = presentation.presentedView,
                  presentation.presentedViewController.presentedViewController == nil else { return false }
            return !sheet.convert(sheet.bounds, to: container).contains(touch.location(in: container))
        }

        func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                               shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer) -> Bool { true }

        @objc private func outsideTap() { close?() }
    }
}
#endif

/// Routes pill links (`caper-mention:`) to the mention card instead of the
/// system; web links (`http(s)`) open in the system browser. A tap on a link
/// is handled by the Text itself; the row's message actions still need a
/// long press. macOS keeps text selection only for messages without person
/// pills or web links, so a click on a link is never taken as a selection.
/// VoiceOver gets one "Open profile for …" action per person.
private struct MentionPillInteraction: ViewModifier {
    let pills: [MentionPill]
    var hasLinks = false
    let cards: MentionCardContext

    @ViewBuilder func body(content: Content) -> some View {
        if pills.isEmpty {
            #if os(macOS)
            if hasLinks { content } else { content.textSelection(.enabled) }
            #else
            content
            #endif
        } else {
            content
                .environment(\.openURL, OpenURLAction { url in
                    // A swipe that starts on a pill or link lifts on it too.
                    if SwipeTapGuard.swiping { return .handled }
                    guard url.scheme == MentionCard.scheme else { return .systemAction }
                    if let id = MentionCard.userID(from: url), let pill = pills.first(where: { $0.id == id }) { cards.open(pill) }
                    return .handled
                })
                #if os(macOS)
                .onContinuousHover(coordinateSpace: CoordinateSpace.named(MentionCard.timelineSpace)) { phase in
                    switch phase {
                    case .active(let location): cards.pointer.location = location
                    case .ended: cards.pointer.location = nil
                    }
                }
                #endif
                .accessibilityActions {
                    ForEach(pills) { pill in
                        Button("Open profile for " + cards.name(pill)) { cards.open(pill) }
                    }
                }
        }
    }
}

/// Profile composition has its own per-person draft; only Send opens a DM.
private struct MentionCardView: View {
    @Bindable var model: AppModel
    @Bindable var draft: ProfileMessageDraft
    let person: MentionCardPerson
    /// Shows the host's block confirmation once the card has closed.
    let requestBlock: (BlockTarget) -> Void
    let close: () -> Void
    @StateObject private var autocomplete = ComposerAutocompleteController()
    @State private var showingEmojiPicker = false
    @State private var insertEmoji: ((String) -> Void)?
    @State private var showingFiles = false
    @State private var uploadsAvailable = false
    @State private var previewFile: URL?
    @State private var pending: String?
    @State private var error: String?
    @State private var status: String?
    @State private var choosingSpace = false
    /// Space ID -> "Invited" or "Member", so a sent or redundant invitation can't be retried.
    @State private var invited: [String: String] = [:]
    @State private var blockRequested = false
    private var blocked: Bool { model.blockedIDs.contains(person.id) }
    private var ownedSpaces: [Space] {
        // Don't offer the open space to someone who is already in it.
        let open: String? = model.detail.flatMap { detail in detail.members.contains(where: { $0.id == person.id }) ? detail.space.id : nil }
        return model.spaces.filter { $0.ownerId == model.account?.id && $0.demo != true && $0.id != open }
    }
    private static let redundantInvitation = ["user already in space": "Member", "user already invited": "Invited"]

    init(model: AppModel, person: MentionCardPerson, requestBlock: @escaping (BlockTarget) -> Void, close: @escaping () -> Void) {
        self.model = model
        self.draft = model.profileMessageDraft(personID: person.id)
        self.person = person
        self.requestBlock = requestBlock
        self.close = close
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                // Identity, then Open conversation and Close at the end, as on web.
                HStack(alignment: .center, spacing: 12) {
                    Avatar(name: person.avatarName, size: 48, avatarID: person.avatarId).accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(person.title).font(CaperTheme.font(15, weight: .semibold)).foregroundStyle(CaperTheme.text).lineLimit(1)
                            .accessibilityIdentifier("mention-card-title")
                        if let subtitle = person.subtitle {
                            Text(subtitle).font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                                .accessibilityIdentifier("mention-card-username")
                        }
                    }
                    Spacer(minLength: 0)
                    HStack(spacing: 2) {
                        if !person.isSelf {
                            Menu {
                                Button(blocked ? "Unblock" : "Block") {
                                    if blocked { Task { await unblock() } }
                                    else { blockRequested = true; close() }
                                }.accessibilityIdentifier("mention-card-block")
                            } label: {
                                // The label must stay an accessibility element: with only
                                // CaperIcon's hidden image, the menu leaves the tree entirely.
                                CaperIcon(name: "ellipsis", size: 16)
                                    .foregroundStyle(CaperTheme.muted)
                                    .frame(width: 32, height: 32).contentShape(Rectangle())
                                    .accessibilityElement(children: .ignore)
                                    .accessibilityLabel("More profile actions")
                            }
                            .menuStyle(.button).buttonStyle(.plain).menuIndicator(.hidden)
                            .fixedSize()
                            .disabled(pending != nil || draft.sending || draft.presentingDialog)
                            .accessibilityLabel("More profile actions")
                            .accessibilityIdentifier("mention-card-more")
                        }
                        if canMessage {
                            CardIconButton(icon: "message-circle", label: pending == "message" ? "Opening…" : "Open conversation") {
                                Task { await openConversation() }
                            }
                            .disabled(pending != nil || draft.sending || draft.presentingDialog)
                            .accessibilityIdentifier("mention-card-message")
                        }
                        CardIconButton(icon: "x", label: "Close profile") { close() }
                            .disabled(draft.sending || draft.presentingDialog)
                            .accessibilityIdentifier("mention-card-close")
                    }
                }
                if person.isSelf {
                    Text("You").font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                        .accessibilityIdentifier("mention-card-you")
                } else {
                    if !blocked, person.username != nil, !ownedSpaces.isEmpty {
                        Button("Invite to space") { choosingSpace.toggle() }
                            .buttonStyle(CaperSecondaryButton(minHeight: 42)).disabled(pending != nil || draft.sending)
                            .accessibilityIdentifier("mention-card-invite")
                    }
                    if choosingSpace, !blocked, person.username != nil {
                        VStack(spacing: 6) {
                            ForEach(ownedSpaces) { space in
                                Button(pending == space.id ? "Sending…" : space.name + (invited[space.id].map { " · \($0)" } ?? "")) {
                                    Task { await invite(space) }
                                }.buttonStyle(CaperSecondaryButton(minHeight: 42))
                                    .disabled(pending != nil || draft.sending || invited[space.id] != nil)
                            }
                        }.accessibilityLabel("Spaces you own")
                    }
                    if let status {
                        Text(status).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("mention-card-status")
                    }
                    if let error {
                        Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("mention-card-error")
                    }
                    if canMessage {
                        // The composer sits below a full-width divider, as on web.
                        Rectangle().fill(CaperTheme.border).frame(height: 1).padding(.horizontal, -16)
                        composer
                    }
                }
            }.padding(16)
        }
        #if os(macOS)
        // A fixed width keeps text wrapping stable; the height fits the
        // content, and only a long space list scrolls.
        .frame(width: person.isSelf ? 280 : 320, alignment: .topLeading)
        .frame(maxHeight: 620)
        .fixedSize(horizontal: false, vertical: true)
        .background(CaperTheme.surface)
        #else
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .presentationDetents(person.isSelf ? [.height(196)] : [.medium, .large])
        .presentationDragIndicator(.visible)
        .presentationBackground(CaperTheme.surface)
        .background(ProfileSheetOutsideTap {
            if pending == nil && !draft.preventsImplicitDismissal { close() }
        })
        #endif
        .interactiveDismissDisabled(draft.preventsImplicitDismissal)
        .fileImporter(isPresented: $showingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            defer { draft.presentingDialog = false }
            switch result {
            case .success(let urls):
                var files: [LocalAttachmentFile] = []
                for url in urls.prefix(AttachmentPolicy.maxAttachments) {
                    do { files.append(try AttachmentStaging.stage(copying: url)) }
                    catch { draft.error = "Some files couldn’t be added." }
                }
                let failed = files.count < min(urls.count, AttachmentPolicy.maxAttachments)
                draft.add(files)
                if failed { draft.error = "Some files couldn’t be added." }
            case .failure(let error):
                if (error as NSError).code != NSUserCancelledError { draft.error = "Files couldn’t be added. Try again." }
            }
        }
        .quickLookPreview($previewFile)
        #if os(macOS)
        .popover(isPresented: $showingEmojiPicker) { emojiPicker.onDisappear { finishEmojiPicker() } }
        #else
        .sheet(isPresented: $showingEmojiPicker, onDismiss: finishEmojiPicker) { emojiPicker }
        #endif
        .onChange(of: showingFiles) { _, _ in updateDialogState() }
        .onChange(of: showingEmojiPicker) { _, _ in updateDialogState() }
        .onChange(of: previewFile) { _, _ in updateDialogState() }
        .onChange(of: draft.editable) { _, editable in if !editable { autocomplete.update(nil) } }
        .onDisappear {
            draft.presentingDialog = false
            if blockRequested {
                requestBlock(BlockTarget(id: person.id, username: person.username, displayName: person.title, avatarId: person.avatarId))
            }
        }
        // Contain, so this identifier does not replace each child's own.
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("mention-card")
    }

    private var canMessage: Bool { !person.isSelf && !blocked && person.username != nil }

    /// The channel composer's shell: Attach, the field, Emoji and Send in one
    /// box, with staged files (image thumbnails included) above it.
    private var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if model.directMessages.contains(where: { $0.peer.id == person.id && $0.status == .incoming }) {
                Text("Open the conversation and accept this message request before replying.")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
            }
            if !draft.files.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(draft.files) { file in
                        StagedFileChip(id: file.id, name: file.local.name,
                                       kind: AttachmentKind.local(contentType: file.local.contentType),
                                       localURL: file.local.url,
                                       status: AttachmentPolicy.formatBytes(file.local.size),
                                       removable: draft.editable,
                                       preview: { draft.presentingDialog = true; previewFile = file.local.url },
                                       remove: { draft.remove(id: file.id) })
                    }
                }.accessibilityLabel("Files to send")
            }
            ComposerSuggestionsView(controller: autocomplete).disabled(!draft.editable || pending != nil)
            HStack(alignment: .bottom, spacing: 0) {
                Button { draft.presentingDialog = true; showingFiles = true } label: {
                    CaperIcon(name: "paperclip", size: 17).foregroundStyle(CaperTheme.muted)
                        .frame(width: 40, height: 42).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(!uploadsAvailable || !draft.editable || pending != nil || draft.files.count >= AttachmentPolicy.maxAttachments)
                .opacity(uploadsAvailable ? 1 : 0.4)
                .help(uploadsAvailable ? "Attach files" : "File uploads are unavailable")
                .accessibilityLabel("Attach files")
                .accessibilityIdentifier("mention-card-files")
                ZStack(alignment: .topLeading) {
                    if draft.text.isEmpty {
                        Text("Message @\(person.username ?? person.title)").font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted)
                            .lineLimit(1).padding(.leading, 11).padding(.top, 12).allowsHitTesting(false).accessibilityHidden(true)
                    }
                    NativeMessageComposer(text: Binding(get: { draft.text }, set: { if draft.editable { draft.text = $0 } }),
                                          placeholder: "Message @\(person.username ?? person.title)", controller: autocomplete, mentions: .empty,
                                          submit: { send() }, editable: draft.editable && pending == nil,
                                          accessibilityID: "mention-card-composer", pasteAttachments: uploadsAvailable ? pasteAttachments : nil,
                                          highlightSource: MentionSource(members: person.username.map { [.member(id: person.id, username: $0, displayName: person.title)] } ?? [], specials: false))
                }
                .frame(minHeight: 42, maxHeight: 140).fixedSize(horizontal: false, vertical: true)
                Button {
                    insertEmoji = autocomplete.insertAction
                    draft.presentingDialog = true; showingEmojiPicker = true
                } label: {
                    CaperIcon(name: "smile", size: 18).foregroundStyle(CaperTheme.muted)
                        .frame(width: 36, height: 42).contentShape(Rectangle())
                }
                .buttonStyle(.plain).disabled(!draft.editable || pending != nil)
                .help("Insert emoji").accessibilityLabel("Insert emoji")
                .accessibilityIdentifier("mention-card-emoji")
                Button { send() } label: {
                    CaperIcon(name: "send", size: 15)
                        .frame(width: 32, height: 32).contentShape(Rectangle())
                }
                .buttonStyle(PrimaryIconButton())
                .disabled(!draft.canSend || pending != nil || draft.presentingDialog)
                .padding(.trailing, 5).padding(.bottom, 5)
                .help(draft.pending == nil ? "Send" : "Retry send")
                .accessibilityLabel(draft.pending == nil ? "Send" : "Retry send")
                .accessibilityIdentifier("mention-card-send")
            }
            .background(CaperTheme.composer).clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
            if draft.pending != nil, !draft.sending {
                Text("Delivery isn’t confirmed. Send retries the same message.")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let progress = draft.progress { Text(progress).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted) }
            if let error = draft.error {
                Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("mention-card-error")
            }
        }
        .task {
            do {
                _ = try await model.api.assetUsage()
                try Task.checkCancellation()
                uploadsAvailable = !draft.discarded
            } catch { uploadsAvailable = false }
        }
    }

    private func send() {
        guard draft.canSend, pending == nil, !draft.presentingDialog else { return }
        Task { if await model.sendProfileMessage(to: person) { close() } }
    }

    private var emojiPicker: some View {
        ReactionPicker(title: "Insert emoji") { emoji in
            if draft.editable { insertEmoji?(emoji) }
            showingEmojiPicker = false
        }
    }

    private func finishEmojiPicker() {
        insertEmoji = nil
        autocomplete.restoreFocus()
    }

    private func updateDialogState() {
        draft.presentingDialog = showingFiles || showingEmojiPicker || previewFile != nil
    }

    private func pasteAttachments() -> Bool {
        guard uploadsAvailable else { return false }
        guard draft.editable, pending == nil else { return true }
        do {
            let files = try NativeAttachmentPaste.stage()
            guard !files.isEmpty else { return false }
            draft.add(files)
        } catch { draft.error = "Pasted files couldn’t be added. Try selecting them with Files." }
        return true
    }

    private func openConversation() async {
        guard pending == nil, !draft.sending, !blocked, let username = person.username else { return }
        pending = "message"; error = nil
        defer { pending = nil }
        // Open an incoming request as a preview; never run the create endpoint
        // for a known request because that endpoint would accept it.
        await model.refreshDirectMessages()
        guard model.account != nil, !draft.discarded else { return }
        if let failure = model.directMessagesError { error = failure; return }
        do {
            if let conversation = model.directMessages.first(where: { $0.peer.id == person.id }) {
                await model.select(directMessage: conversation)
                close()
            } else if try await model.createDirectMessage(username: username) { close() }
            else { error = "Couldn’t open that conversation." }
        } catch { self.error = FriendlyError.message(for: error) }
    }

    private func invite(_ space: Space) async {
        guard pending == nil, !draft.sending, !blocked, invited[space.id] == nil, let username = person.username else { return }
        pending = space.id; error = nil; status = nil
        defer { pending = nil }
        do {
            try await model.addSpaceMember(username: username, spaceID: space.id)
            invited[space.id] = "Invited"
            status = "Invitation sent to \(space.name)."
        } catch {
            // Redundant invitations still mark the row so it isn't retried.
            if let already = (error as? APIError).flatMap({ Self.redundantInvitation[$0.message] }) { invited[space.id] = already }
            self.error = FriendlyError.message(for: error)
        }
    }

    /// Unblock keeps the card open and says so, as on web.
    private func unblock() async {
        guard pending == nil, !draft.sending else { return }
        pending = "unblock"; error = nil
        defer { pending = nil }
        do {
            try await model.unblock(accountID: person.id)
            status = "Unblocked."
        } catch { self.error = FriendlyError.message(for: error) }
    }
}

private struct ReactionRow: View {
    let message: ChatMessage
    @Bindable var chat: ChatModel
    let reactors: ReactorContext

    var body: some View {
        ReactionFlowLayout(spacing: 6) {
            ForEach(message.reactions ?? []) { reaction in
                ReactionChip(message: message, reaction: reaction, chat: chat, reactors: reactors)
            }
        }
        // Preserve individual reaction-chip identifiers inside the flow layout.
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("reaction-row-\(message.id)")
    }
}

private struct ReactionChip: View {
    let message: ChatMessage
    let reaction: MessageReaction
    @Bindable var chat: ChatModel
    let reactors: ReactorContext
    #if os(iOS)
    /// The release that ends a hold must not also toggle the reaction.
    @State private var suppressTap = false
    #else
    @State private var hovering = false
    @State private var tooltipVisible = false
    @FocusState private var focused: Bool
    #endif
    private var own: Bool { reactors.viewerID.map { reaction.authorIds.contains($0) } ?? false }

    /// Names once loaded; until then, or after a failure, the snapshot count.
    private var summary: String {
        let name = EmojiArtwork.name(for: reaction.emoji)
        if case .loaded(let people) = chat.reactorsState(for: message, emoji: reaction.emoji, viewerID: reactors.viewerID),
           !people.isEmpty {
            return ReactionSummary.text(authors: people, selfID: reactors.viewerID, emojiName: name, emoji: reaction.emoji)
        }
        return ReactionSummary.fallback(authorIDs: reaction.authorIds, selfID: reactors.viewerID, emojiName: name, emoji: reaction.emoji)
    }

    #if os(macOS)
    /// Read while the body is evaluated, so loading names updates the tooltip.
    private var tooltipText: String? { tooltipVisible ? summary : nil }
    #endif

    var body: some View {
        Button {
            #if os(iOS)
            if suppressTap { suppressTap = false; return }
            #endif
            guard !SwipeTapGuard.swiping else { return }
            Task { await chat.setReaction(messageID: message.id, emoji: reaction.emoji, active: !own) }
        } label: {
            HStack(spacing: 4) {
                EmojiArtworkView(emoji: reaction.emoji, size: 18)
                Text("\(reaction.authorIds.count)").font(CaperTheme.font(11, weight: .bold))
            }
            .padding(.horizontal, 7).frame(height: 28)
            // Filled in the border's shape: a plain fill's square corners showed outside it.
            .background(own ? CaperTheme.terracotta.opacity(0.24) : CaperTheme.surface, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(own ? CaperTheme.terracottaBright : CaperTheme.border))
            #if os(iOS)
            // A 44pt touch target around the chip, which is drawn at its usual size, as on web.
            .frame(minHeight: 44)
            .contentShape(Rectangle())
            #endif
        }
        .buttonStyle(.plain)
        #if os(macOS)
        .focused($focused)
        .modifier(ControlHover(isFocused: focused))
        #else
        .modifier(ControlHover())
        #endif
        .disabled(!chat.canInteract)
        .accessibilityLabel("\(reaction.emoji) reaction, \(reaction.authorIds.count), \(own ? "selected by you" : "not selected by you")")
        .accessibilityAddTraits(own ? .isSelected : [])
        // Outside .disabled: read-only previews can still see who reacted.
        .accessibilityAction(named: Text("Show who reacted")) { reactors.show(message.id, reaction.emoji) }
        #if os(iOS)
        // Just under the row's 0.5 s hold, so the chip's sheet wins over
        // message actions; the row's handler also yields while it is open.
        .simultaneousGesture(LongPressGesture(minimumDuration: 0.45).onEnded { _ in
            suppressTap = true
            reactors.show(message.id, reaction.emoji)
        })
        // A hold whose touch was cancelled never delivers the suppressed tap.
        .onChange(of: reactors.sheetOpen) { _, open in if !open { suppressTap = false } }
        #else
        .onHover { hovering = $0 }
        .task(id: hovering) {
            guard hovering else { tooltipVisible = false; return }
            do { try await Task.sleep(for: .milliseconds(400)) } catch { return }
            tooltipVisible = true
            _ = chat.requestReactors(messageID: message.id)
        }
        .onChange(of: message.reactionSeq) { _, _ in
            if tooltipVisible { _ = chat.requestReactors(messageID: message.id) }
        }
        .anchorPreference(key: ReactionTooltipKey.self, value: .bounds) { [text = tooltipText, emoji = reaction.emoji] anchor in
            text.map { ReactionTooltip(emoji: emoji, text: $0, anchor: anchor) }
        }
        #endif
    }
}

/// How chips reveal who reacted: the viewer (shown as "You") and the
/// press-and-hold sheet, which a chip must know has closed.
private struct ReactorContext {
    let viewerID: String?
    let sheetOpen: Bool
    let show: (_ messageID: String, _ emoji: String) -> Void
}

private struct ReactorsTarget: Identifiable, Equatable {
    let messageID: String
    let emoji: String
    var id: String { "\(messageID)|\(emoji)" }
}

#if os(macOS)
private struct ReactionTooltip {
    let emoji: String
    let text: String
    let anchor: Anchor<CGRect>
}

#if os(macOS)
/// A composer box whose suggestions the surrounding conversation draws.
private protocol ComposerAnchorKey: PreferenceKey where Value == Anchor<CGRect>? {}
extension ComposerAnchorKey {
    static var defaultValue: Anchor<CGRect>? { nil }
    static func reduce(value: inout Anchor<CGRect>?, nextValue: () -> Anchor<CGRect>?) { value = value ?? nextValue() }
}
private enum ChatComposerAnchorKey: ComposerAnchorKey {}
private enum ThreadComposerAnchorKey: ComposerAnchorKey {}

/// Mac suggestions cover history above the composer without resizing it or
/// triggering bottom-follow. They are drawn by the whole conversation, not as
/// an overflowing overlay on the composer: clicks and accessibility hit-testing
/// only reach views inside their parent's frame.
private struct ComposerSuggestionsOverlay<Key: ComposerAnchorKey>: ViewModifier {
    @ObservedObject var controller: ComposerAutocompleteController
    func body(content: Content) -> some View {
        content.overlayPreferenceValue(Key.self) { anchor in
            // Only while suggestions show, so no layer covers the conversation otherwise.
            if let anchor, controller.match != nil {
                GeometryReader { proxy in
                    let box = proxy[anchor]
                    ComposerSuggestionsView(controller: controller)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(width: box.width)
                        .alignmentGuide(.leading) { _ in -box.minX }
                        .alignmentGuide(.top) { size in -(box.minY - 6 - size.height) }
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                }
            }
        }
    }
}
#endif

private struct ReactionTooltipKey: PreferenceKey {
    static var defaultValue: ReactionTooltip? { nil }
    static func reduce(value: inout ReactionTooltip?, nextValue: () -> ReactionTooltip?) {
        value = value ?? nextValue()
    }
}

/// Discord-style hover card: a large emoji beside who reacted.
private struct ReactionTooltipBubble: View {
    let emoji: String
    let text: String
    var body: some View {
        HStack(spacing: 10) {
            if EmojiArtwork.entry(for: emoji) != nil {
                EmojiArtworkView(emoji: emoji, size: 38)
            } else {
                Text(emoji).font(.system(size: 32))
            }
            Text(text).font(CaperTheme.font(12, weight: .medium)).foregroundStyle(CaperTheme.text)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(10)
        .background(CaperTheme.blackout, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .shadow(color: .black.opacity(0.35), radius: 8, y: 3)
        .frame(maxWidth: 280, alignment: .leading)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(text)
        .accessibilityIdentifier("reaction-tooltip")
    }
}
#endif

/// Who reacted, by emoji: tabs in chip order, then each person.
private struct ReactorsSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Bindable var chat: ChatModel
    let messageID: String
    let viewerID: String?
    @State private var selected: String

    init(chat: ChatModel, messageID: String, emoji: String, viewerID: String?) {
        self.chat = chat
        self.messageID = messageID
        self.viewerID = viewerID
        _selected = State(initialValue: emoji)
    }

    private var message: ChatMessage? { chat.messages.first { $0.id == messageID } }
    private var reactions: [MessageReaction] { message?.reactions ?? [] }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                tabs
                Rectangle().fill(CaperTheme.border).frame(height: 1)
                people
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .background(CaperTheme.raised)
            // Contain, so this identifier does not replace each tab's and row's own.
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("reactors-sheet")
            .navigationTitle("Reactions")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
        #if os(iOS)
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .presentationBackground(CaperTheme.raised)
        #else
        .frame(minWidth: 360, minHeight: 420)
        #endif
        // Opening again after a change, or a change while open, refetches.
        .task(id: message?.reactionSeq) { _ = chat.requestReactors(messageID: messageID) }
        .onChange(of: reactions.map(\.emoji)) { _, emojis in
            guard let first = emojis.first else { dismiss(); return }
            if !emojis.contains(selected) { selected = first }
        }
    }

    private var tabs: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 4) {
                    ForEach(reactions) { reaction in
                        let isSelected = reaction.emoji == selected
                        let count = reaction.authorIds.count
                        Button { selected = reaction.emoji } label: {
                            HStack(spacing: 5) {
                                EmojiArtworkView(emoji: reaction.emoji, size: 20)
                                Text("\(count)").font(CaperTheme.font(12, weight: .bold))
                            }
                            .foregroundStyle(isSelected ? CaperTheme.text : CaperTheme.muted)
                            .padding(.horizontal, 10)
                            .frame(minHeight: 44)
                            .overlay(alignment: .bottom) {
                                Rectangle().fill(isSelected ? CaperTheme.terracotta : Color.clear).frame(height: 2)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .modifier(ControlHover())
                        .id(reaction.emoji)
                        .accessibilityLabel("\(reaction.emoji), \(count) \(count == 1 ? "person" : "people")")
                        .accessibilityAddTraits(isSelected ? .isSelected : [])
                        .accessibilityIdentifier("reactors-tab-\(reaction.emoji)")
                    }
                }
                .padding(.horizontal, 12)
            }
            .onAppear { proxy.scrollTo(selected, anchor: .center) }
            .onChange(of: selected) { _, emoji in withAnimation { proxy.scrollTo(emoji, anchor: .center) } }
        }
    }

    @ViewBuilder private var people: some View {
        if let message, let reaction = reactions.first(where: { $0.emoji == selected }) {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    Text(ReactionSummary.emojiLabel(emojiName: EmojiArtwork.name(for: reaction.emoji), emoji: reaction.emoji))
                        .font(CaperTheme.font(12, weight: .medium)).foregroundStyle(CaperTheme.muted)
                        .padding(.horizontal, 16).padding(.top, 14).padding(.bottom, 6)
                        .accessibilityIdentifier("reactors-emoji-name")
                    switch chat.reactorsState(for: message, emoji: reaction.emoji, viewerID: viewerID) {
                    case .loaded(let reactors):
                        ForEach(reactors) { person in ReactorRow(person: person) }
                    case .loading:
                        HStack(spacing: 8) {
                            ProgressView().controlSize(.small)
                            Text("Loading…").font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                        }
                        .padding(16)
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("reactors-loading")
                    case .failed:
                        VStack(alignment: .leading, spacing: 10) {
                            Text("Couldn’t load reactions").font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                            Button("Retry") { _ = chat.requestReactors(messageID: messageID, force: true) }
                                .buttonStyle(CaperSecondaryButton())
                                .accessibilityIdentifier("reactors-retry")
                        }
                        .padding(16)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}

private struct ReactorRow: View {
    let person: ReactorPerson
    var body: some View {
        HStack(spacing: 10) {
            Avatar(name: person.name, size: 34, avatarID: person.avatarId)
            VStack(alignment: .leading, spacing: 2) {
                Text(person.name).font(CaperTheme.font(14, weight: .bold)).foregroundStyle(CaperTheme.text).lineLimit(1)
                if let username = person.username, !username.isEmpty {
                    Text("@\(username)").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16).padding(.vertical, 7)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("reactor-row-\(person.id)")
    }
}

struct ReactionFlowLayout: Layout {
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

#if os(iOS)
/// Parity tests only. XCUITest's keyboard element excludes the prediction bar
/// on iOS 26; UIKit reports the full system keyboard in screen coordinates.
/// This measures the system boundary, not the composer's own layout.
private struct ParityKeyboardProbe: View {
    @State private var top: CGFloat?

    var body: some View {
        if ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity" {
            Color.clear.frame(width: 1, height: 1)
                .allowsHitTesting(false)
                .accessibilityElement()
                .accessibilityIdentifier("parity-keyboard-top")
                .accessibilityLabel("System keyboard top")
                .accessibilityValue(top.map { String(Double($0)) } ?? "")
                .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardDidChangeFrameNotification)) { notification in
                    top = (notification.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue.minY
                }
        }
    }
}

/// Parity tests only. Since iOS 26 the UI test runner is not authorized to
/// read a pasteboard item another app wrote (PBErrorDomain code 13), so the app
/// that wrote it reads its own pasteboard back and exposes the exact string.
private struct ParityPasteboardProbe: View {
    @State private var copied = ""

    var body: some View {
        Color.clear.frame(width: 1, height: 1)
            .accessibilityElement()
            .accessibilityLabel(copied)
            .accessibilityIdentifier("parity-pasteboard")
            .onReceive(NotificationCenter.default.publisher(for: UIPasteboard.changedNotification)) { _ in
                copied = UIPasteboard.general.string ?? ""
            }
    }
}

private struct MessageActionsSheet: View {
    @Environment(\.dismiss) private var dismiss
    let message: ChatMessage
    @Bindable var chat: ChatModel
    @Binding var showingEmojiPicker: Bool
    @State private var editing = false
    @State private var history = false
    let canReact: Bool
    let canPin: Bool
    let togglePin: () -> Void
    let reply: (() -> Void)?
    var inThread = false
    let canForward: Bool
    let forward: () -> Void
    let quickReaction: (String) -> Void
    let selectReaction: (String) -> Void
    /// Block the author, offered for other signed-in accounts. The parent
    /// confirms once this sheet has gone, since one view presents at a time.
    var block: (() -> Void)? = nil
    /// Unblock an author you already blocked, on a message you chose to show.
    var unblock: (() -> Void)? = nil
    /// Who reacted, from the first reaction's tab; opened once this sheet has gone.
    var viewReactions: ((String) -> Void)? = nil
    @State private var blockRequested = false
    /// Forward also waits for this sheet to go, so its picker can present.
    @State private var forwardRequested = false
    @State private var reactionsRequested: String?

    private let quickReactions = MessageQuickReactions.emoji

    /// Fits every offered action without scrolling.
    private var actionsHeight: CGFloat {
        var height: CGFloat = 286
        if reply != nil { height += 44 }
        if canForward { height += 50 }
        if chat.canEdit(message) { height += 44 }
        if message.forward == nil && (message.revision ?? 1) > 1 { height += 44 }
        if viewReactions != nil && message.reactions?.isEmpty == false { height += 44 }
        if block != nil || unblock != nil { height += 44 }
        if message.content.text.isEmpty { height -= 44 }
        return height
    }

    private var quickReactionRow: some View {
        HStack(spacing: 4) {
            ForEach(quickReactions, id: \.self) { emoji in
                Button { quickReaction(emoji) } label: {
                    EmojiArtworkView(emoji: emoji, size: 28).frame(maxWidth: .infinity, minHeight: 44)
                }
                .buttonStyle(.plain)
                .disabled(!canReact)
                .accessibilityLabel("React with \(emoji)")
                .accessibilityIdentifier("quick-reaction-\(emoji)")
            }
            Button { showingEmojiPicker = true } label: {
                CaperIcon(name: "smile-plus", size: 24)
                    .frame(maxWidth: .infinity, minHeight: 44)
            }
            .buttonStyle(.plain)
            .disabled(!canReact)
            .accessibilityLabel("Add reaction")
            .accessibilityIdentifier("message-action-add-reaction")
        }
    }

    private var actionList: some View {
        VStack(spacing: 0) {
            Button {
                chat.reply(to: message, inThread: inThread)
                dismiss()
            } label: { CaperLabel("Reply", icon: "reply") }.frame(maxWidth: .infinity, minHeight: 44, alignment: .leading).disabled(!chat.canInteract || chat.pendingMessage != nil)
            Divider()
            if chat.canEdit(message) {
                Button { editing = true } label: { CaperLabel("Edit message", icon: "pencil") }.frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                Divider()
            }
            if message.forward == nil && (message.revision ?? 1) > 1 {
                Button { history = true } label: { CaperLabel("Message history", icon: "history") }.frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                Divider()
            }
            if viewReactions != nil, let reaction = message.reactions?.first {
                Button {
                    reactionsRequested = reaction.emoji
                    dismiss()
                } label: {
                    CaperLabel("View reactions", icon: "users").frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                }.accessibilityIdentifier("message-action-view-reactions")
                Divider()
            }
            Button(action: togglePin) {
                CaperLabel(message.pin == nil ? "Pin message" : "Unpin message", icon: message.pin == nil ? "pin" : "pin-off")
                    .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            }.disabled(!canPin)
            if let reply {
                Divider()
                Button(action: reply) {
                    Label { Text("Reply in thread") } icon: { CaperIcon(name: "message-square") }
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                }
            }
            if canForward {
                Divider()
                Button {
                    forwardRequested = true
                    dismiss()
                } label: {
                    CaperLabel("Forward message", icon: "forward").frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                }
            }
            // Attachment-only messages have no text to copy.
            if !message.content.text.isEmpty {
                Divider()
                Button {
                    UIPasteboard.general.string = message.content.text
                    dismiss()
                } label: {
                    CaperLabel("Copy text", icon: "copy").frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                }
            }
            Divider()
            Button {
                UIPasteboard.general.string = message.id
                dismiss()
            } label: {
                CaperLabel("Copy message ID", icon: "hash").frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            }
            if block != nil {
                Divider()
                Button(role: .destructive) {
                    blockRequested = true
                    dismiss()
                } label: {
                    CaperLabel("Block \(message.author.name)", icon: "ban").frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                        .foregroundStyle(CaperTheme.terracottaBright)
                }.accessibilityIdentifier("message-action-block")
            } else if let unblock {
                Divider()
                Button {
                    unblock()
                    dismiss()
                } label: {
                    CaperLabel("Unblock \(message.author.name)", icon: "ban").frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                }.accessibilityIdentifier("message-action-unblock")
            }
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 12)
        .background(CaperTheme.surface, in: RoundedRectangle(cornerRadius: 8))
    }

    // The sheet is split into properties and its height is built in typed
    // steps: as one expression, Xcode 16's type checker gave up on the body.
    var body: some View {
        Group {
            if showingEmojiPicker {
                ReactionPicker(select: selectReaction)
            } else {
                VStack(spacing: 16) {
                    quickReactionRow
                    actionList
                }
                .padding(16)
                .font(CaperTheme.font(15))
                // Contain, so the sheet's identifier does not replace each
                // action's own (iOS 27 applies it to every child button).
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("message-actions-sheet")
            }
        }
        .presentationBackground(CaperTheme.raised)
        .presentationDetents(showingEmojiPicker ? [.medium, .large] : [.height(actionsHeight), .large])
        .onDisappear {
            if blockRequested { block?() }
            if forwardRequested { forward() }
            if let emoji = reactionsRequested { viewReactions?(emoji) }
        }
        .presentationDragIndicator(.visible)
        .sheet(isPresented: $editing) { MessageEditorView(chat: chat, message: message) { editing = false; dismiss() } }
        .sheet(isPresented: $history) { MessageHistoryView(chat: chat, message: message) { history = false } }
        .onChange(of: chat.editingContext) { _, _ in editing = false; history = false; dismiss() }
    }
}
#endif

private struct PinnedMessagesView: View {
    @Bindable var chat: ChatModel
    let model: AppModel
    let close: () -> Void
    @State private var actionTarget: ChatMessage?
    @State private var showingEmojiPicker = false
    @State private var reactorsTarget: ReactorsTarget?
    @State private var mentionCard: MentionCardTarget?
    @State private var mentionPointer = MentionPointer()
    #if os(macOS)
    @Environment(\.mediaViewer) private var mediaViewer
    #endif
    private var viewerID: String? { chat.currentAuthor?.id ?? model.account?.id }
    private var mentionCards: MentionCardContext {
        MentionCardContext(pointer: mentionPointer, name: { model.mentionPerson($0, viewerID: viewerID).title }, open: { pill in
            actionTarget = nil; showingEmojiPicker = false; reactorsTarget = nil
            mentionCard = MentionCardTarget(pill: pill, pointer: mentionPointer.location)
        }, openAuthor: { author in
            if case .author(let shown)? = mentionCard?.reference, shown.id == author.id { return }
            actionTarget = nil; showingEmojiPicker = false; reactorsTarget = nil
            mentionCard = MentionCardTarget(author: author, pointer: mentionPointer.location)
        })
    }
    var body: some View {
        VStack(spacing: 0) {
            // Web: one close control, an X, at the trailing edge.
            HStack {
                Spacer()
                Button(action: close) { CaperIcon(name: "x", size: 18) }
                    .buttonStyle(SidebarIconButton())
                    .help("Close pins")
                    .accessibilityLabel("Close pins")
                    .accessibilityIdentifier("close-pins")
            }.padding(.horizontal, 12).frame(height: 50)
                // Centred on the bar.
                .overlay { Text("Pins").font(CaperTheme.font(15, weight: .bold)).accessibilityAddTraits(.isHeader) }
                .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
            if let error = chat.jumpError { Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error).padding(12) }
            if chat.displayedPins.isEmpty {
                ContentUnavailableView("No pinned messages", systemImage: "pin", description: Text("Pinned messages in this channel will appear here."))
            } else {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(chat.displayedPins) { message in
                            // The row's own bottom padding already separates it from Go to message.
                            VStack(alignment: .leading, spacing: 0) {
                                MessageRow(message: message, chat: chat, currentUserID: viewerID,
                                    reactors: ReactorContext(viewerID: chat.currentAuthor?.id, sheetOpen: reactorsTarget != nil) { id, emoji in reactorsTarget = ReactorsTarget(messageID: id, emoji: emoji) },
                                    mentionCards: mentionCards, inThread: true, inPins: true, forwardOpened: close) { actionTarget = message }
                                Button {
                                    mentionCard = nil
                                    Task { if await chat.goToMessage(message) { close() } }
                                } label: {
                                    HStack(spacing: 6) {
                                        Text(chat.jumpingToMessage ? "Loading message…" : "Go to message")
                                        CaperIcon(name: "arrow-right", size: 14)
                                    }
                                }.buttonStyle(.plain).font(CaperTheme.font(12, weight: .medium))
                                    .foregroundStyle(CaperTheme.muted).disabled(chat.jumpingToMessage)
                                    .frame(minHeight: 44).padding(.leading, 62).padding(.bottom, 8)
                                    .accessibilityIdentifier("go-to-message-\(message.id)")
                            }.frame(maxWidth: .infinity, alignment: .leading)
                                .overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
                                .accessibilityIdentifier("pinned-message-\(message.id)")
                        }
                    }
                }
            }
        }.background(CaperTheme.conversation)
            .coordinateSpace(.named(MentionCard.timelineSpace))
            .modifier(MentionCardHost(target: $mentionCard, model: model, viewerID: viewerID))
            .sheet(item: $reactorsTarget) { target in ReactorsSheet(chat: chat, messageID: target.messageID, emoji: target.emoji, viewerID: chat.currentAuthor?.id) }
            #if os(iOS)
            .sheet(item: $actionTarget) { message in
                MessageActionsSheet(message: message, chat: chat, showingEmojiPicker: $showingEmojiPicker,
                    canReact: chat.canInteract,
                    canPin: chat.canInteract && !chat.pendingPins.contains(message.id),
                    togglePin: { actionTarget = nil; Task { await chat.setPin(messageID: message.id, active: message.pin == nil) } },
                    reply: { actionTarget = nil; close(); Task { await chat.openThread(message.threadRootId ?? message.id) } },
                    canForward: chat.canForward, forward: { actionTarget = nil; close(); chat.forwardTarget = message },
                    quickReaction: { emoji in
                        let active = !(message.reactions?.first(where: { $0.emoji == emoji })?.authorIds.contains(chat.currentAuthor?.id ?? "") ?? false)
                        actionTarget = nil; Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: active) }
                    }, selectReaction: { emoji in actionTarget = nil; Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: true) } },
                    viewReactions: { emoji in reactorsTarget = ReactorsTarget(messageID: message.id, emoji: emoji) })
            }
            #else
            .popover(item: $actionTarget) { message in ReactionPicker { emoji in actionTarget = nil; Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: true) } } }
            #endif
            .onChange(of: chat.editingContext) { _, _ in actionTarget = nil; reactorsTarget = nil; close() }
            .onChange(of: chat.replyTargets) { _, _ in actionTarget = nil; close() }
            .onChange(of: chat.focusRevision) { _, _ in close() }
            #if os(iOS)
            .modifier(MediaViewerHost())
            #else
            // The popover closes so the window's viewer fills the window.
            .environment(\.mediaViewer, mediaViewer.map { show in MediaViewerAction { close(); show($0) } })
            #endif
            .accessibilityIdentifier("pinned-messages")
    }
}

/// A muted icon control in a card header (Open conversation, Close).
private struct CardIconButton: View {
    /// A shared Lucide glyph (`CaperIcon`); else `systemImage`, where the catalog has none yet.
    var icon: String? = nil
    var systemImage = ""
    let label: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            glyph.foregroundStyle(CaperTheme.muted)
                .frame(width: 32, height: 32)
                #if os(iOS)
                .frame(minWidth: 44, minHeight: 44)
                #endif
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .modifier(ControlHover())
        .help(label)
        .accessibilityLabel(label)
    }

    @ViewBuilder private var glyph: some View {
        if let icon { CaperIcon(name: icon, size: 15) }
        else { Image(systemName: systemImage).font(.system(size: 15, weight: .medium)) }
    }
}

/// A composer's emoji control: the emoji picker, inserting at the text view's
/// selection when it opened. The `:name:` shortcut still works.
private struct ComposerEmojiButton: View {
    let controller: ComposerAutocompleteController
    var enabled = true
    @State private var showing = false
    @State private var insert: ((String) -> Void)?

    var body: some View {
        Button {
            insert = controller.insertAction
            showing = true
        } label: {
            CaperIcon(name: "smile", size: 18).foregroundStyle(CaperTheme.muted)
                .frame(width: 36, height: 42).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .help("Insert emoji")
        .accessibilityLabel("Insert emoji")
        .accessibilityIdentifier("composer-emoji")
        #if os(macOS)
        .popover(isPresented: $showing) { picker.onDisappear(perform: finish) }
        #else
        .sheet(isPresented: $showing, onDismiss: finish) { picker }
        #endif
    }

    private var picker: some View {
        ReactionPicker(title: "Insert emoji") { emoji in
            insert?(emoji)
            showing = false
        }
    }

    private func finish() {
        insert = nil
        controller.restoreFocus()
    }
}

private struct ReactionPicker: View {
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @State private var category = "smileys_people"
    private let categories = [
        ("smileys_people", "Smileys & people", "😀"), ("animals_nature", "Animals & nature", "🐱"),
        ("food_drink", "Food & drink", "🍔"), ("travel_places", "Travel & places", "🚌"),
        ("activities", "Activities", "⚽"), ("objects", "Objects", "👕"),
        ("symbols", "Symbols", "🎵"), ("flags", "Flags", "🏁")
    ]
    #if os(macOS)
    @FocusState private var focusedEmoji: String?
    #endif
    var title = "Add reaction"
    let select: (String) -> Void
    private var choices: [EmojiCatalogEntry] {
        let term = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !term.isEmpty else { return EmojiArtwork.pickerChoices.filter { $0.category == category } }
        return EmojiArtwork.pickerChoices.filter { $0.name.lowercased().contains(term) || $0.keywords.lowercased().contains(term) || $0.category.replacingOccurrences(of: "_", with: " ").contains(term) }
    }
    var body: some View {
        #if os(macOS)
        // NavigationStack's cancellation toolbar is not shown in a macOS
        // popover. Keep both the header and accessible bounds in its content.
        VStack(spacing: 0) {
            HStack {
                Text(title).font(CaperTheme.font(14, weight: .bold))
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
            }.padding(.horizontal, 12).padding(.top, 12)
            content
        }
        .frame(width: 352, height: 420)
        .accessibilityElement(children: .contain)
        // Containers otherwise report only their children's union, which
        // shrinks when the scrolling catalog becomes an empty-search message.
        .contentShape(.accessibility, Rectangle())
        .accessibilityIdentifier("reaction-picker")
        #else
        NavigationStack {
            content
                .navigationTitle(title)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { dismiss() }
                    }
                }
        }
        .frame(minWidth: 320, minHeight: 420)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("reaction-picker")
        #endif
    }
    private var content: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                ForEach(categories, id: \.0) { item in
                    let active = query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && category == item.0
                    Button { category = item.0; query = "" } label: {
                        VStack(spacing: 0) {
                            EmojiArtworkView(emoji: item.2, size: 22).frame(maxWidth: .infinity, minHeight: 42)
                            Rectangle().fill(active ? CaperTheme.terracottaBright : .clear).frame(height: 2)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .modifier(ControlHover())
                    .accessibilityLabel(item.1)
                    .accessibilityIdentifier("emoji-category-\(item.0)")
                    .accessibilityAddTraits(active ? .isSelected : [])
                }
            }.padding(.horizontal, 8)
            TextField("Search emoji", text: $query)
                .textFieldStyle(.roundedBorder)
                .accessibilityIdentifier("reaction-picker-search")
                .padding(12)
            Text(query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? categories.first { $0.0 == category }!.1 : "Search results")
                .font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 12)
            if choices.isEmpty {
                ContentUnavailableView("No emoji found", systemImage: "magnifyingglass", description: Text("Try another search."))
                    .accessibilityIdentifier("reaction-picker-empty")
            } else {
                ScrollView {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 42), spacing: 8)], spacing: 8) {
                        ForEach(choices) { entry in
                            Button { select(entry.emoji) } label: {
                                EmojiArtworkView(emoji: entry.emoji, size: 30).frame(width: 42, height: 42)
                            }
                            .buttonStyle(.plain)
                            #if os(macOS)
                            .focused($focusedEmoji, equals: entry.id)
                            .modifier(ControlHover(isFocused: focusedEmoji == entry.id))
                            #else
                            .modifier(ControlHover())
                            #endif
                            .accessibilityLabel(entry.name)
                        }
                    }.padding(12)
                }.id("\(category):\(query)").accessibilityIdentifier("reaction-picker-grid")
            }
        }
    }
}

private struct NativeReplyContext: View {
    let reference: MessageReply
    let motionKey: String
    @Bindable var chat: ChatModel
    var replyGap: CGFloat = 6
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var expanded = false
    @State private var fullHeight: CGFloat = 0
    @State private var collapsedHeight: CGFloat = 0
    @State private var buttonHeight: CGFloat = 0
    @State private var previousY: CGFloat?
    @State private var displacement: CGFloat = 0
    @State private var velocity: CGFloat = 0
    @State private var motion: Task<Void, Never>?
    private var hidden: Bool { chat.blockedAuthorIDs.contains(reference.author.id) && reference.author.id != chat.viewerID }
    private var excerpt: String {
        let text = reference.fullText ?? reference.text
        return text.isEmpty ? (reference.hasAttachments ? "Attachment" : "Message") : text
    }
    private var preview: String { String(String.UnicodeScalarView(excerpt.unicodeScalars.prefix(100))) }
    var body: some View {
        // Match the other clients; seed the reply, not the original it quotes.
        let seed = motionKey.utf8.reduce(UInt32(2166136261)) { ($0 ^ UInt32($1)) &* 16777619 }
        let stiffness = 140 + CGFloat(seed & 255) / 255 * 100
        let damping = 11 + CGFloat((seed >> 8) & 255) / 255 * 7
        let impulse = 1 + CGFloat((seed >> 16) & 255) / 255 * 0.4
        let limit = 5 + CGFloat(seed >> 24) / 255 * 2
        let speedLimit = limit * 16
        let canExpand = !hidden && (expanded || preview != excerpt || fullHeight > collapsedHeight + 1)
        #if os(iOS)
        let previewMinHeight: CGFloat = 54
        #else
        let previewMinHeight: CGFloat = 36
        #endif
        ZStack(alignment: .bottom) {
            Button { Task { _ = await chat.goToReply(reference) } } label: {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        if hidden { Text("Blocked message").lineLimit(1) }
                        else {
                            Avatar(name: reference.author.name, size: 16, avatarID: reference.author.avatarId)
                            Text(reference.author.name).fontWeight(.bold).lineLimit(1)
                            if reference.hasAttachments { CaperIcon(name: "paperclip", size: 13) }
                        }
                        Spacer(minLength: 4)
                        Text("View original").font(CaperTheme.font(11)).fixedSize()
                    }.frame(minHeight: 20)
                    if !hidden {
                        Text(expanded ? excerpt : preview + (preview != excerpt ? "…" : "")).lineLimit(expanded ? nil : 3).fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { if !expanded { collapsedHeight = $0 } }
                            .frame(minHeight: !expanded && canExpand ? previewMinHeight : 0, alignment: .topLeading)
                            .overlay {
                                if !expanded && (preview != excerpt || fullHeight > collapsedHeight + 1) {
                                    LinearGradient(stops: [
                                        .init(color: CaperTheme.surface.opacity(0), location: 0),
                                        .init(color: CaperTheme.surface.opacity(0), location: 0.4),
                                        .init(color: CaperTheme.surface, location: 1),
                                    ], startPoint: .top, endPoint: .bottom).allowsHitTesting(false).accessibilityHidden(true)
                                }
                            }
                            .background {
                                Text(excerpt).lineLimit(nil).fixedSize(horizontal: false, vertical: true)
                                    .frame(maxWidth: .infinity, alignment: .leading).hidden().accessibilityHidden(true)
                                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { fullHeight = $0 }
                            }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                #if os(iOS)
                .frame(minHeight: 44, alignment: .topLeading)
                #else
                .frame(minHeight: 24, alignment: .topLeading)
                #endif
            }.buttonStyle(.plain)
                .accessibilityLabel(hidden ? "View original blocked message" : "View original message from \(reference.author.name)")
                .accessibilityHint(hidden ? "" : excerpt)
                .padding(.bottom, expanded && canExpand ? buttonHeight + 6 : 0)
            if canExpand {
                Button(expanded ? "Show less" : "Show more") { expanded.toggle() }
                    .buttonStyle(ReplyExpandButton())
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { buttonHeight = $0 }
                    .frame(maxWidth: .infinity, alignment: .center)
                    .offset(y: expanded ? 0 : buttonHeight / 2 - max(collapsedHeight, previewMinHeight) * 0.3)
                    .accessibilityValue(expanded ? "Expanded" : "Collapsed")
            }
        }.padding(.horizontal, 12).padding(.vertical, 10)
            .background(RoundedRectangle(cornerRadius: 8).fill(CaperTheme.surface))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border.opacity(0.7), lineWidth: 1))
            .padding(.leading, 44).padding(.bottom, 13)
            .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
            .overlay(alignment: .topLeading) {
                GeometryReader { geometry in
                    if !hidden {
                        Canvas { context, _ in
                            let bottom = geometry.size.height + replyGap + 17
                            let angle = displacement * 0.025
                            let dx = 12 * cos(angle), dy = 12 * sin(angle)
                            let start = CGPoint(x: -dx, y: bottom + dy)
                            let end = CGPoint(x: 56 - dx, y: 20 - dy)
                            let span = start.y - end.y
                            let wire = Path { path in
                                path.move(to: start)
                                path.addCurve(to: end,
                                    control1: CGPoint(x: start.x - 7 + displacement, y: start.y - span * 0.3),
                                    control2: CGPoint(x: end.x - 30 + displacement, y: end.y + span * 0.3))
                            }
                            var string = context
                            string.translateBy(x: 20, y: 0)
                            string.stroke(wire, with: .color(CaperTheme.muted.opacity(0.75)), style: StrokeStyle(lineWidth: 1.1, lineCap: .round))
                            drawSocket(context, at: CGPoint(x: 56, y: 20), angle: angle)
                            drawSocket(context, at: CGPoint(x: 0, y: bottom), angle: -angle)
                        }
                        // Canvas clips to its bounds; reserve the gutter and the avatar below.
                        .frame(width: 78, height: geometry.size.height + 45)
                        .offset(x: -20)
                    }
                }.allowsHitTesting(false).accessibilityHidden(true)
            }
            .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).minY } action: { y in
                let delta = (previousY ?? y) - y
                previousY = y
                guard !reduceMotion && !hidden && delta != 0 else { return }
                velocity = max(-speedLimit, min(speedLimit, velocity + delta * impulse))
                guard motion == nil else { return }
                motion = Task { @MainActor in
                    var previousTime = Date()
                    while !Task.isCancelled {
                        do { try await Task.sleep(for: .milliseconds(16)) } catch { return }
                        let now = Date()
                        let dt = CGFloat(min(now.timeIntervalSince(previousTime), 0.032))
                        previousTime = now
                        velocity -= (stiffness * displacement + damping * velocity) * dt
                        displacement = max(-limit, min(limit, displacement + velocity * dt))
                        if abs(displacement) < 0.01 && abs(velocity) < 0.05 { break }
                    }
                    displacement = 0; velocity = 0; motion = nil
                }
            }
            .onChange(of: reduceMotion) { _, _ in stopMotion() }
            .onChange(of: hidden) { _, _ in stopMotion() }
            .onChange(of: motionKey) { _, _ in stopMotion(); previousY = nil }
            .onDisappear { stopMotion(); previousY = nil }
    }

    private func stopMotion() {
        motion?.cancel(); motion = nil; displacement = 0; velocity = 0
    }

    private func drawSocket(_ context: GraphicsContext, at position: CGPoint, angle: CGFloat) {
        var socket = context
        socket.translateBy(x: position.x + 20, y: position.y)
        socket.rotate(by: .radians(Double(angle)))
        let cup = Path { path in
            path.move(to: CGPoint(x: -12, y: -2.4))
            path.addQuadCurve(to: CGPoint(x: -12, y: 2.4), control: CGPoint(x: -13, y: 0))
            path.addLine(to: CGPoint(x: -1, y: 4.5)); path.addLine(to: CGPoint(x: -1, y: -4.5)); path.closeSubpath()
        }
        let metal = Gradient(stops: [
            .init(color: Color(red: 100/255, green: 107/255, blue: 112/255), location: 0),
            .init(color: Color(red: 162/255, green: 169/255, blue: 173/255), location: 0.32),
            .init(color: Color(red: 112/255, green: 120/255, blue: 125/255), location: 0.55),
            .init(color: Color(red: 48/255, green: 54/255, blue: 58/255), location: 1),
        ])
        socket.fill(cup, with: .linearGradient(metal, startPoint: CGPoint(x: 0, y: -4.5), endPoint: CGPoint(x: 0, y: 4.5)))
        socket.stroke(cup, with: .color(CaperTheme.border), lineWidth: 0.6)
        let highlight = Path { $0.move(to: CGPoint(x: -11, y: -1.6)); $0.addLine(to: CGPoint(x: -3, y: -3)) }
        socket.stroke(highlight, with: .color(CaperTheme.text.opacity(0.35)), lineWidth: 0.6)
        let rim = Path(ellipseIn: CGRect(x: -2.6, y: -4.5, width: 3.2, height: 9))
        socket.fill(rim, with: .color(Color(red: 37/255, green: 42/255, blue: 45/255)))
        socket.stroke(rim, with: .color(CaperTheme.terracotta), lineWidth: 0.8)
        socket.fill(Path(ellipseIn: CGRect(x: -1.7, y: -3.4, width: 1.4, height: 6.8)), with: .color(CaperTheme.surface))
    }
}

private struct ReplyExpandButton: ButtonStyle {
    @State private var hovered = false
    @Environment(\.isFocused) private var focused
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.blackout)
            .padding(.horizontal, 14).padding(.vertical, 6)
            #if os(iOS)
            .frame(minHeight: 44)
            #else
            .frame(minHeight: 32)
            #endif
            .background(configuration.isPressed ? CaperTheme.text.opacity(0.85) : hovered ? Color.white : CaperTheme.text)
            .clipShape(Capsule())
            .overlay(Capsule().stroke(focused ? CaperTheme.terracotta : .clear, lineWidth: 2))
            .onHover { hovered = $0 }.modifier(ControlPointer())
    }
}

private struct NativeReplyDraft: View {
    let reference: MessageReply
    @Bindable var chat: ChatModel
    let root: String
    private var hidden: Bool { chat.blockedAuthorIDs.contains(reference.author.id) && reference.author.id != chat.viewerID }
    var body: some View {
        HStack(alignment: .top, spacing: 7) {
            VStack(alignment: .leading, spacing: 3) {
                Text("Replying to \(hidden ? "blocked message" : reference.author.name)")
                    .fontWeight(.bold).lineLimit(1)
                if !hidden {
                    let text = reference.fullText ?? reference.text
                    Text(text.isEmpty ? (reference.hasAttachments ? "Attachment" : "Message") : text)
                        .lineLimit(2).fixedSize(horizontal: false, vertical: true)
                }
            }.frame(maxWidth: .infinity, alignment: .leading)
            CardIconButton(icon: "x", label: "Cancel reply") { chat.replyTargets[root] = nil }
        }.padding(.leading, 10).padding(.vertical, 8)
            .overlay(alignment: .leading) { Rectangle().fill(CaperTheme.terracotta).frame(width: 2) }
            .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
    }
}

private struct PendingMessageRow: View {
    let pending: PendingMessage; let author: ChatAuthor?; let error: String?
    let rejected: Bool; let canEdit: Bool
    var chat: ChatModel? = nil
    var uploadFailed = false
    /// Under the author's own previous row: no avatar or name, as `MessageRow`.
    var grouped = false
    let retry: () -> Void; let edit: () -> Void; let dismiss: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
        if let reference = pending.replyTo, let chat { NativeReplyContext(reference: reference, motionKey: pending.id, chat: chat, replyGap: 5) }
        HStack(alignment: .top, spacing: 10) {
            if grouped {
                // Keep the avatar column empty; a clear name still reaches VoiceOver.
                Text(author?.name ?? "Guest").font(CaperTheme.font(10)).lineLimit(1)
                    .foregroundStyle(Color.clear).frame(width: 34)
            } else {
                Avatar(name: author?.name ?? "Guest", size: 34, avatarID: author?.avatarId)
            }
            VStack(alignment: .leading, spacing: 4) {
                if !grouped { Text(author?.name ?? "Guest").font(CaperTheme.font(13, weight: .bold)) }
                if !pending.text.isEmpty {
                    Text(pending.text).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                if !pending.attachments.isEmpty {
                    // Local copies until the server's signed URLs arrive.
                    MessageAttachmentsView(attachments: pending.attachments, chat: nil).opacity(0.7)
                }
                if let error {
                    VStack(alignment: .leading, spacing: 5) {
                        Text(uploadFailed ? error : "\(rejected ? "Not sent." : "Not confirmed yet.") \(error)")
                        HStack(spacing: 12) {
                            if rejected {
                                Button("Edit", action: edit).disabled(!canEdit)
                                    .help(canEdit ? "" : "Clear your current draft to edit this message.")
                                Button("Dismiss", action: dismiss)
                            } else { Button(uploadFailed ? "Retry" : "Retry send", action: retry) }
                        }
                    }
                    .font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.error)
                }
            }
        }
        }.frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 18).padding(.top, grouped ? 2 : 10).padding(.bottom, 10)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("pending-message-\(pending.id)")
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
                    Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                        .fixedSize(horizontal: false, vertical: true)
                }
                // The sign-in page's action style, as on web.
                Button { Task { await model.saveProfile(username: username, displayName: displayName) } } label: {
                    LoginActionLabel(title: "Finish account", busyTitle: "Saving…", busy: model.busy)
                }
                    .buttonStyle(LoginActionButton(minWidth: 200))
                    .disabled(model.busy || ProfileValidation.error(username: username, displayName: displayName) != nil)
                    .accessibilityLabel(model.busy ? "Saving…" : "Finish account")
                    .accessibilityIdentifier("profile-continue")
                    .frame(maxWidth: .infinity, alignment: .trailing)
                HStack {
                    Spacer()
                    Button("Log out") { Task { await model.logout() } }.buttonStyle(.plain)
                        .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).modifier(ControlHover())
                }
            }.padding(28).frame(maxWidth: 440).frame(maxWidth: .infinity)
        }.background(CaperTheme.blackout)
    }
}

#if os(macOS)
/// The Settings scene is also available before signing in, through Caper → Settings.
@MainActor public struct CaperSettingsView: View {
    @State private var loginItem = CaperLoginItem.shared
    @Bindable private var effects = CaperEffects.shared
    @Bindable private var notes: WhatsNewModel
    public init(whatsNew: WhatsNewModel) { notes = whatsNew }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Text("Settings").font(CaperTheme.font(20, weight: .bold))
                VStack(alignment: .leading, spacing: 14) {
                    Text("Startup").font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                    Toggle(isOn: Binding(get: { loginItem.registered }, set: { loginItem.setEnabled($0) })) {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("Launch at login").font(CaperTheme.font(14))
                            Text("Open Caper when you sign in to your computer.")
                                .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                        }
                    }.toggleStyle(.switch).disabled(!loginItem.available)
                        .accessibilityIdentifier("launch-at-login")
                    if loginItem.status == .requiresApproval {
                        Text("Launch at login needs approval in System Settings.")
                            .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                        Button("Open Login Items Settings…") { SMAppService.openSystemSettingsLoginItems() }
                    }
                    if let error = loginItem.error {
                        Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                        Button("Dismiss startup error") { loginItem.error = nil }
                    }
                }
                Rectangle().fill(CaperTheme.border).frame(height: 1)
                VStack(alignment: .leading, spacing: 14) {
                    Text("Sounds").font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                    Toggle(isOn: $effects.soundsEnabled) {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("Caper sound effects").font(CaperTheme.font(14))
                            Text("Play sounds for messages and app interactions.")
                                .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                        }
                    }.toggleStyle(.switch).accessibilityIdentifier("sound-effects")
                }
                Rectangle().fill(CaperTheme.border).frame(height: 1)
                Button("What’s new") { notes.open() }.accessibilityIdentifier("open-whats-new")
            }.padding(24).frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(width: 500, height: 380)
        .background(CaperTheme.surface)
        .foregroundStyle(CaperTheme.text)
        .preferredColorScheme(.dark)
        .tint(CaperTheme.terracotta)
        .buttonStyle(CaperSecondaryButton())
        .accessibilityIdentifier("desktop-settings")
        .onAppear { CaperFontLoader.register(); loginItem.refresh(); notes.settingsOpen = true }
        .onDisappear { notes.settingsOpen = false }
        .sheet(item: Binding(get: { notes.settingsOpen ? notes.presentation : nil }, set: { notes.presentation = $0 })) { display in
            WhatsNewSheet(notes: notes, display: notes.presentation ?? display)
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in loginItem.refresh() }
    }
}
#endif

private struct WorkspaceSheetView: View {
    let item: WorkspaceSheet
    @Bindable var model: AppModel
    /// Replaces this dialog with another, e.g. Create a space → Join with invite link.
    var show: (WorkspaceSheet) -> Void = { _ in }
    let close: () -> Void
    var body: some View {
        Group {
            switch item {
            case .login: EmptyView() // Presented full-screen by LoginPresentation.
            case .profile: ProfileSheet(model: model, close: close)
            case .createSpace: SpaceEditor(model: model, close: close, managing: false, joinInvite: { show(.joinInvite(nil)) })
            case .joinInvite(let opened): JoinInviteSheet(model: model, opened: opened, close: close)
            case .createChannel: ChannelEditor(model: model, channel: nil, close: close)
            case .newDirectMessage: NewDirectMessageSheet(model: model, close: close)
            case .manageSpace: SpaceEditor(model: model, close: close, managing: true)
            case .manageChannel(let channel): ChannelEditor(model: model, channel: channel, close: close)
            case .invitation(let space): InvitationSheet(model: model, invitation: space, close: close)
            case .channelInvitation(let invitation): ChannelInvitationSheet(model: model, invitation: invitation, close: close)
            case .leaveSpace: ConfirmationSheet(title: "Leave \(model.detail?.space.name ?? "space")?", detail: "You will lose access to its channels and conversations. An owner can add you again later.", action: "Leave space", pendingLabel: "Leaving…", close: close) { try await model.leaveCurrentSpace() }
            case .audio: AudioPreferencesView(voice: model.voice, debugEnabled: model.account?.debugEnabled == true, close: close)
            case .connection: ScrollView { ConnectionDetailsView(voice: model.voice, close: close) }
            case .privacy: PrivacySheet(model: model, close: close)
            case .notifications: NotificationsSheet(model: model, close: close)
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

// MARK: Message requests and blocking

/// "Message requests" with a count of conversations, at the top of the DM
/// section. Requests never show unread dots or count toward unread.
private struct MessageRequestsRow: View {
    @Bindable var model: AppModel
    let narrow: Bool
    let rowHeight: CGFloat
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        let count = model.messageRequests.count
        Button { if !SwipeTapGuard.swiping { model.showingMessageRequests.toggle() } } label: {
            HStack(spacing: 9) {
                // Turns like the Channels chevron instead of swapping icons.
                CaperIcon(name: "chevron-down", size: 14)
                    .rotationEffect(.degrees(!narrow && model.showingMessageRequests ? 0 : -90))
                    .animation(reduceMotion ? nil : .easeOut(duration: 0.15), value: model.showingMessageRequests)
                    .frame(width: 24)
                Text("Message requests").lineLimit(1)
                Spacer()
                Text("\(count)").font(CaperTheme.font(10, weight: .bold)).foregroundStyle(CaperTheme.text)
                    .padding(.horizontal, 6).frame(minWidth: 20, minHeight: 18)
                    .background(CaperTheme.raised, in: Capsule())
                    .overlay(Capsule().stroke(CaperTheme.border))
            }.font(CaperTheme.font(13, weight: .medium)).foregroundStyle(CaperTheme.muted)
                .padding(.horizontal, 9).frame(height: rowHeight)
                .contentShape(Rectangle())
        }.buttonStyle(.plain).modifier(ControlHover())
            .accessibilityLabel("Message requests")
            .accessibilityValue("\(count) \(count == 1 ? "request" : "requests")")
            .accessibilityHint(narrow ? "Opens the list of requests" : model.showingMessageRequests ? "Collapses the list" : "Expands the list")
            .accessibilityIdentifier("message-requests")
    }
}

/// "Feedback" for the feedback team, at the top of the DM section, with the
/// count of loaded reports still waiting for a team reply.
private struct FeedbackRow: View {
    @Bindable var model: AppModel
    let narrow: Bool
    let rowHeight: CGFloat
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        // The archived list has no waiting count.
        let waiting = model.showingArchivedFeedback ? 0 : FeedbackInbox.waitingCount(model.feedbackReports)
        Button {
            guard !SwipeTapGuard.swiping else { return }
            // Phones push the inbox; sidebars expand it in place.
            if narrow { model.showingFeedback = true } else { model.showingFeedback.toggle() }
        } label: {
            HStack(spacing: 9) {
                CaperIcon(name: "chevron-down", size: 14)
                    .rotationEffect(.degrees(!narrow && model.showingFeedback ? 0 : -90))
                    .animation(reduceMotion ? nil : .easeOut(duration: 0.15), value: model.showingFeedback)
                    .frame(width: 24)
                Text(FeedbackInbox.title).lineLimit(1)
                Spacer()
                if waiting > 0 {
                    Text("\(waiting)").font(CaperTheme.font(10, weight: .bold)).foregroundStyle(CaperTheme.text)
                        .padding(.horizontal, 6).frame(minWidth: 20, minHeight: 18)
                        .background(CaperTheme.raised, in: Capsule())
                        .overlay(Capsule().stroke(CaperTheme.border))
                }
            }.font(CaperTheme.font(13, weight: .medium))
                .foregroundStyle(model.visit != nil ? CaperTheme.text : CaperTheme.muted)
                .padding(.horizontal, 9).frame(height: rowHeight)
                .contentShape(Rectangle())
        }.buttonStyle(.plain).modifier(ControlHover())
            .accessibilityLabel(FeedbackInbox.title)
            .accessibilityValue(waiting > 0 ? "\(waiting) waiting" : "")
            .accessibilityHint(narrow ? "Opens the feedback inbox" : model.showingFeedback ? "Collapses the inbox" : "Expands the inbox")
            .accessibilityIdentifier("feedback-inbox")
    }
}

/// Pull to refresh for the pushed feedback inbox on phones.
private struct FeedbackPullToRefresh: ViewModifier {
    let enabled: Bool
    let refresh: () async -> Void
    @ViewBuilder func body(content: Content) -> some View {
        #if os(iOS)
        if enabled { content.refreshable { await refresh() } } else { content }
        #else
        content
        #endif
    }
}

/// The feedback team's inbox: nested under its row in a sidebar, or a pushed
/// page in Browse on narrow layouts. Newest first; a row visits its
/// conversation at the reported message, with its thread open. "Show
/// archived" switches it to the threads the team archived.
private struct FeedbackInboxList: View {
    @Bindable var model: AppModel
    let pushed: Bool
    private var archived: Bool { model.showingArchivedFeedback }
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            if pushed {
                HStack(spacing: 6) {
                    Button { model.showingFeedback = false } label: {
                        HStack(spacing: 4) {
                            CaperIcon(name: "arrow-left", size: 16)
                            Text("Direct messages")
                        }.font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                            .frame(minHeight: 44).contentShape(Rectangle())
                    }.buttonStyle(.plain).modifier(ControlHover()).accessibilityIdentifier("feedback-back")
                    Spacer()
                }
                Text(archived ? FeedbackInbox.archivedTitle : FeedbackInbox.title)
                    .font(CaperTheme.font(15, weight: .bold)).padding(.bottom, 4)
                if !archived {
                    Text(FeedbackInbox.subtitle)
                        .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).padding(.bottom, 8)
                        .fixedSize(horizontal: false, vertical: true)
                }
            } else if archived {
                Text(FeedbackInbox.archivedTitle).font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                    .padding(.horizontal, 9).padding(.vertical, 4)
            }
            if let error = model.feedbackError {
                HStack(spacing: 8) {
                    Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                    Button("Retry") { Task { await model.refreshFeedback() } }.buttonStyle(.plain)
                        .font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.text).modifier(ControlHover())
                        .disabled(model.feedbackLoading)
                        .accessibilityIdentifier("feedback-retry")
                }.padding(.vertical, 6)
            }
            if model.feedbackReports.isEmpty {
                if !model.feedbackLoaded && model.feedbackLoading {
                    Text("Loading…").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted).padding(.vertical, 8)
                } else if model.feedbackLoaded {
                    Text(archived ? FeedbackInbox.archivedEmpty : FeedbackInbox.empty)
                        .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted).padding(.vertical, 8)
                }
            }
            // Relative times stay current while the inbox is open.
            TimelineView(.everyMinute) { context in
                VStack(spacing: 2) {
                    ForEach(model.feedbackReports) { report in
                        FeedbackReportRow(model: model, report: report, pushed: pushed, archived: archived, now: context.date)
                    }
                }
            }
            if model.feedbackCursor != nil {
                Button(model.feedbackLoading ? "Loading…" : "Load older feedback") { Task { await model.loadMoreFeedback() } }
                    .buttonStyle(.plain).font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.muted)
                    .frame(minHeight: 32).contentShape(Rectangle()).modifier(ControlHover())
                    .disabled(model.feedbackLoading)
                    .accessibilityIdentifier("feedback-load-older")
            }
            // Quiet, at the bottom: archived threads, and back.
            Button(archived ? FeedbackInbox.backToFeedback : FeedbackInbox.showArchived) {
                Task { await model.showArchivedFeedback(!archived) }
            }
                .buttonStyle(.plain).font(CaperTheme.font(11, weight: .medium)).foregroundStyle(CaperTheme.muted)
                #if os(iOS)
                .frame(minHeight: 44)
                #else
                .frame(minHeight: 28)
                #endif
                .padding(.horizontal, pushed ? 0 : 9)
                .contentShape(Rectangle()).modifier(ControlHover())
                .accessibilityIdentifier("feedback-show-archived")
        }
        .padding(.horizontal, pushed ? 16 : 0).padding(.leading, pushed ? 0 : 16)
        .padding(.vertical, pushed ? 8 : 0)
        .frame(maxWidth: .infinity, alignment: .leading)
        // Refreshed each time it opens.
        .task { await model.refreshFeedback() }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("feedback-list")
    }
}

/// One report: who asked, where, what, when, and (in the inbox) whether the
/// team answered. Archive (Restore in the archived list) is a hover button
/// on macOS and a context menu (press and hold on iPhone) everywhere.
private struct FeedbackReportRow: View {
    @Bindable var model: AppModel
    let report: FeedbackReport
    let pushed: Bool
    let archived: Bool
    let now: Date
    #if os(macOS)
    @State private var hovered = false
    @FocusState private var actionFocused: Bool
    #endif
    private var selected: Bool { model.visit?.channelID == report.channel.id && model.visit?.reportID == report.messageId }
    private var actionLabel: String { archived ? FeedbackInbox.restore : FeedbackInbox.archiveLabel(report) }
    private func toggleArchive() {
        Task { await model.setFeedbackArchived(report, archived: !archived) }
    }
    var body: some View {
        HStack(alignment: .top, spacing: 2) {
            rowButton
            #if os(macOS)
            Button(action: toggleArchive) {
                CaperIcon(name: archived ? "archive-restore" : "archive", size: 15)
                    .frame(width: 28, height: 28).contentShape(Rectangle())
            }.buttonStyle(.plain).foregroundStyle(CaperTheme.muted)
                .focused($actionFocused)
                .opacity(hovered || actionFocused ? 1 : 0)
                .allowsHitTesting(hovered || actionFocused)
                .modifier(ControlHover(isFocused: actionFocused))
                .help(actionLabel).accessibilityLabel(actionLabel)
                .accessibilityIdentifier("feedback-archive-\(report.id)")
            #endif
        }
        #if os(macOS)
        .onHover { hovered = $0 }
        #endif
        .contextMenu {
            Button(action: toggleArchive) {
                Label { Text(actionLabel) } icon: { caperMenuIcon(archived ? "archive-restore" : "archive") }
            }
        }
    }
    private var rowButton: some View {
        let preview = FeedbackInbox.preview(report)
        return Button { Task { await model.openFeedback(report) } } label: {
            HStack(alignment: .top, spacing: 9) {
                Avatar(name: report.reporter.displayName, size: pushed ? 32 : 22, avatarID: report.reporter.avatarId)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(report.reporter.displayName).font(CaperTheme.font(13, weight: .medium))
                            .foregroundStyle(CaperTheme.text).lineLimit(1)
                        Spacer(minLength: 4)
                        Text(FeedbackInbox.relativeTime(report.createdAt, now: now)).font(CaperTheme.font(10))
                            .foregroundStyle(CaperTheme.muted).fixedSize()
                    }
                    Text(FeedbackInbox.location(report)).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                    if !preview.isEmpty {
                        Text(preview).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.text).lineLimit(2)
                            .multilineTextAlignment(.leading)
                    }
                    if !archived {
                        Text(FeedbackInbox.status(report)).font(CaperTheme.font(10, weight: .bold))
                            .foregroundStyle(report.answered ? CaperTheme.muted : CaperTheme.terracottaBright)
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 9).padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
            #if os(iOS)
            .frame(minHeight: 44)
            #endif
            .background(selected ? CaperTheme.terracotta.opacity(0.16) : Color.clear)
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .contentShape(Rectangle())
        }.buttonStyle(.plain).modifier(ControlHover())
            .accessibilityLabel(FeedbackInbox.accessibilityLabel(report, now: now, archived: archived))
            .accessibilityValue(selected ? "Selected" : "")
            .accessibilityIdentifier("feedback-report-\(report.id)")
            .accessibilityAction(named: Text(actionLabel), toggleArchive)
    }
}

/// Marks a Caper admin's (feedback team member's) message, in the pin gold
/// with a soft glow. The server stamps it, so a look-alike name can't claim it.
struct AdminBadge: View {
    var body: some View {
        Text("ADMIN").font(CaperTheme.font(9, weight: .bold)).foregroundStyle(CaperTheme.pinGold)
            .padding(.horizontal, 5)
            .background(CaperTheme.pinGold.opacity(0.1), in: RoundedRectangle(cornerRadius: 4))
            .overlay(RoundedRectangle(cornerRadius: 4).stroke(CaperTheme.pinGold.opacity(0.55)))
            .shadow(color: CaperTheme.pinGold.opacity(0.45), radius: 4)
            .accessibilityLabel("Admin")
    }
}

/// Incoming requests: nested under the row in a sidebar, or a pushed page on
/// narrow layouts. Opening one reads it by id and never accepts it.
private struct MessageRequestsList: View {
    @Bindable var model: AppModel
    let pushed: Bool
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            if pushed {
                HStack(spacing: 6) {
                    Button { model.showingMessageRequests = false } label: {
                        HStack(spacing: 4) {
                            CaperIcon(name: "arrow-left", size: 16)
                            Text("Direct messages")
                        }.font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.muted)
                            .frame(minHeight: 44).contentShape(Rectangle())
                    }.buttonStyle(.plain).modifier(ControlHover()).accessibilityIdentifier("message-requests-back")
                    Spacer()
                }
                Text("Message requests").font(CaperTheme.font(15, weight: .bold)).padding(.bottom, 4)
                Text("From people you don't share a space with. Accept to reply.")
                    .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).padding(.bottom, 8)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if model.messageRequests.isEmpty {
                Text("No message requests.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted).padding(.vertical, 8)
            }
            ForEach(model.messageRequests) { request in
                let selected = model.selectedDirectMessageID == request.id
                Button { Task { await model.select(directMessage: request) } } label: {
                    HStack(spacing: 9) {
                        Avatar(name: request.peer.displayName, size: pushed ? 32 : 22, avatarID: request.peer.avatarId)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(request.peer.displayName).font(CaperTheme.font(13, weight: .medium)).lineLimit(1)
                            Text("@\(request.peer.username)").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                        }
                        Spacer(minLength: 0)
                    }.foregroundStyle(selected ? CaperTheme.text : CaperTheme.muted)
                        .padding(.horizontal, 9).padding(.vertical, 5)
                        #if os(iOS)
                        .frame(minHeight: 44)
                        #endif
                        .background(selected ? CaperTheme.terracotta.opacity(0.16) : Color.clear)
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                        .contentShape(Rectangle())
                }.buttonStyle(.plain)
                    .modifier(NavigationPrefetchModifier { model.prefetch(directMessage: request) })
                    .accessibilityLabel("\(request.peer.displayName), @\(request.peer.username)")
                    .accessibilityValue(selected ? "Selected" : "")
                    .accessibilityIdentifier("message-request-\(request.id)")
            }
        }
        .padding(.horizontal, pushed ? 16 : 0).padding(.leading, pushed ? 0 : 16)
        .padding(.vertical, pushed ? 8 : 0)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("message-requests-list")
    }
}

/// Replaces the composer in an incoming request.
private struct MessageRequestBar: View {
    @Bindable var model: AppModel
    let conversation: DirectMessageConversation
    let block: () -> Void
    @State private var pending: String?
    @State private var error: String?
    var body: some View {
        let prompt = MessageRequests.prompt(for: conversation.peer)
        VStack(alignment: .leading, spacing: 10) {
            (Text(prompt.name).font(CaperTheme.font(13, weight: .bold)) + Text(prompt.detail).font(CaperTheme.font(13)))
                .foregroundStyle(CaperTheme.text).fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("message-request-prompt")
            HStack(spacing: 8) {
                Button(pending == "accept" ? "Accepting…" : "Accept") { run("accept") { try await model.acceptRequest(conversation) } }
                    .buttonStyle(CaperPrimaryButton()).frame(maxWidth: 160)
                    .accessibilityIdentifier("message-request-accept")
                Button(pending == "decline" ? "Declining…" : "Decline") { run("decline") { try await model.declineRequest(conversation) } }
                    .buttonStyle(CaperSecondaryButton(minHeight: 42))
                    .accessibilityIdentifier("message-request-decline")
                Button("Block", role: .destructive, action: block).buttonStyle(CaperSecondaryButton(minHeight: 42))
                    .accessibilityIdentifier("message-request-block")
                Spacer(minLength: 0)
            }.disabled(pending != nil)
            if let error {
                Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                    .accessibilityIdentifier("message-request-error")
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("message-request-bar")
    }

    private func run(_ action: String, _ operation: @escaping @MainActor () async throws -> Void) {
        pending = action; error = nil
        Task { @MainActor in
            do { try await operation() } catch { self.error = FriendlyError.message(for: error) }
            pending = nil
        }
    }
}

/// Replaces the composer in a DM whose peer you blocked.
private struct BlockedConversationBar: View {
    let conversation: DirectMessageConversation
    let unblock: () -> Void
    var body: some View {
        HStack(spacing: 10) {
            Text(MessageRequests.blockedNotice(for: conversation.peer)).font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
            Spacer(minLength: 0)
            Button("Unblock", action: unblock).buttonStyle(CaperSecondaryButton())
                .accessibilityIdentifier("dm-blocked-unblock")
        }
        .frame(maxWidth: .infinity, minHeight: 42, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("dm-blocked-bar")
    }
}

/// (Ban icon) "2 blocked messages — Show": a run of blocked authors' messages.
private struct BlockedRunRow: View {
    let run: BlockedRun
    let toggle: () -> Void
    var body: some View {
        HStack(spacing: 6) {
            CaperIcon(name: "ban", size: 16).foregroundStyle(CaperTheme.muted)
            Text("\(run.label) —").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                .accessibilityHidden(true)
            Button(run.revealed ? "Hide" : "Show", action: toggle).buttonStyle(.plain)
                .font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.terracottaBright)
                .frame(minHeight: 32).contentShape(Rectangle())
                .modifier(ControlPointer())
                .accessibilityLabel("\(run.revealed ? "Hide" : "Show") \(run.label)")
                .accessibilityIdentifier("blocked-run-toggle")
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 18).padding(.vertical, 4)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("blocked-run-\(run.messages.first?.id ?? "")")
    }
}

/// "Block Alex?" before any block. Unblocking needs no confirmation.
private struct BlockConfirmation: ViewModifier {
    @Binding var target: BlockTarget?
    let block: (BlockTarget) async -> Void
    func body(content: Content) -> some View {
        content.alert(target?.confirmationTitle ?? "Block?",
                      isPresented: Binding(get: { target != nil }, set: { if !$0 { target = nil } }),
                      presenting: target) { target in
            Button("Block", role: .destructive) { Task { await block(target) } }
            Button("Cancel", role: .cancel) {}
        } message: { _ in
            Text(BlockTarget.confirmationMessage)
        }
    }
}

/// Account privacy: who can start a DM with you, blocked accounts, and how to delete your account.
private struct PrivacySheet: View {
    @Bindable var model: AppModel
    let close: () -> Void
    @State private var unblocking: Set<String> = []
    @State private var unblockError: String?
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: "Privacy", detail: "Message requests and blocked accounts.", closeLabel: "Close privacy settings", close: close)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    Text("Who can start a DM with you").font(CaperTheme.font(13, weight: .bold))
                    VStack(spacing: 0) {
                        ForEach(DirectMessagePrivacy.allCases) { option in
                            let selected = model.directMessagePrivacy == option
                            Button { Task { await model.setDirectMessagePrivacy(option) } } label: {
                                HStack(alignment: .top, spacing: 10) {
                                    Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                                        .foregroundStyle(selected ? CaperTheme.terracottaBright : CaperTheme.muted)
                                        .font(.system(size: 15)).padding(.top, 1)
                                    VStack(alignment: .leading, spacing: 3) {
                                        Text(option.title).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.text)
                                        if let detail = option.detail {
                                            Text(detail).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                                                .fixedSize(horizontal: false, vertical: true)
                                        }
                                    }
                                    Spacer(minLength: 0)
                                }.padding(.vertical, 8).frame(minHeight: 44).contentShape(Rectangle())
                            }.buttonStyle(.plain).disabled(model.directMessagePrivacy == nil)
                                .modifier(ControlPointer())
                                .accessibilityAddTraits(selected ? .isSelected : [])
                                .accessibilityIdentifier("dm-privacy-\(option.rawValue)")
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel("Who can start a DM with you")
                    if model.directMessagePrivacy == nil && model.privacyError == nil {
                        Text("Loading…").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    }
                    if let error = model.privacyError {
                        Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                            .accessibilityIdentifier("dm-privacy-error")
                    }
                    Rectangle().fill(CaperTheme.border).frame(height: 1).padding(.vertical, 6)
                    Text("Blocked accounts").font(CaperTheme.font(13, weight: .bold))
                    if model.blockedAccounts.isEmpty {
                        Text(model.blocksLoaded ? "You haven't blocked anyone." : model.blocksError ?? "Loading…")
                            .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                            .accessibilityIdentifier("blocked-accounts-empty")
                    }
                    ForEach(model.blockedAccounts) { account in
                        HStack(spacing: 10) {
                            Avatar(name: account.displayName, size: 30, avatarID: account.avatarId)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(account.displayName).font(CaperTheme.font(13, weight: .medium)).lineLimit(1)
                                if !account.username.isEmpty {
                                    Text("@\(account.username)").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).lineLimit(1)
                                }
                            }
                            Spacer(minLength: 0)
                            Button(unblocking.contains(account.id) ? "Unblocking…" : "Unblock") { unblock(account.id) }
                                .buttonStyle(CaperSecondaryButton()).disabled(unblocking.contains(account.id))
                                .accessibilityLabel("Unblock \(account.displayName)")
                                .accessibilityIdentifier("unblock-\(account.id)")
                        }.frame(minHeight: 44)
                            .accessibilityElement(children: .contain)
                            .accessibilityIdentifier("blocked-account-\(account.id)")
                    }
                    if let unblockError {
                        Text(unblockError).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                    }
                    Rectangle().fill(CaperTheme.border).frame(height: 1).padding(.vertical, 6)
                    Text("Delete account").font(CaperTheme.font(13, weight: .bold))
                    Link(destination: PrivacySettings.deleteAccountURL) {
                        Text("How to delete your account").font(CaperTheme.font(13)).underline()
                            .foregroundStyle(CaperTheme.text)
                    }
                    .frame(minHeight: 44).modifier(ControlPointer())
                    .accessibilityIdentifier("delete-account-link")
                }.padding(22).frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .background(CaperTheme.surface)
        .accessibilityIdentifier("privacy-settings")
        .task {
            await model.loadPrivacy()
            await model.refreshBlocks()
        }
    }

    private func unblock(_ id: String) {
        unblocking.insert(id); unblockError = nil
        Task { @MainActor in
            do { try await model.unblock(accountID: id) } catch { unblockError = FriendlyError.message(for: error) }
            unblocking.remove(id)
        }
    }
}

/// One radio choice in a settings sheet, like the DM privacy options.
private struct SettingsChoiceRow: View {
    let title: String
    let selected: Bool
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                    .foregroundStyle(selected ? CaperTheme.terracottaBright : CaperTheme.muted)
                    .font(.system(size: 15)).padding(.top, 1)
                Text(title).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.text)
                Spacer(minLength: 0)
            }.padding(.vertical, 8).frame(minHeight: 44).contentShape(Rectangle())
        }.buttonStyle(.plain)
            // The pointing hand, like the DM privacy choices.
            .modifier(ControlPointer())
            .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// Account notifications: what notifies you, and on an iPhone with push
/// available, this phone's delivery and its on/off switch.
private struct NotificationsSheet: View {
    @Bindable var model: AppModel
    let close: () -> Void
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: NotificationLabels.notifications, detail: "What Caper notifies you about.",
                        closeLabel: "Close notification settings", close: close)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    levelSection
                    if model.pushAvailable { phoneSection }
                    status
                }.padding(22).frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .background(CaperTheme.surface)
        .accessibilityIdentifier("notification-settings")
        .task { await model.loadNotificationSettings() }
    }

    private var levelSection: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(NotificationLabels.notifyMeAbout).font(CaperTheme.font(13, weight: .bold)).padding(.bottom, 6)
            ForEach(NotificationLevel.allCases) { level in
                SettingsChoiceRow(title: level.accountTitle, selected: model.notificationSettings?.level == level) {
                    Task { await model.setAccountNotificationLevel(level) }
                }
                .disabled(model.notificationSettings == nil)
                .accessibilityIdentifier("notify-level-\(level.rawValue)")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(NotificationLabels.notifyMeAbout)
    }

    private var phoneSection: some View {
        VStack(alignment: .leading, spacing: 0) {
            Rectangle().fill(CaperTheme.border).frame(height: 1).padding(.bottom, 14)
            Text(NotificationLabels.sendToThisPhone).font(CaperTheme.font(13, weight: .bold)).padding(.bottom, 6)
            ForEach(MobilePushPolicy.allCases) { policy in
                SettingsChoiceRow(title: policy.title, selected: model.notificationSettings?.mobile == policy) {
                    Task { await model.setMobilePushPolicy(policy) }
                }
                .disabled(model.notificationSettings == nil)
                .accessibilityIdentifier("notify-mobile-\(policy.rawValue)")
            }
            Toggle("Notifications on this phone", isOn: pushBinding)
                .font(CaperTheme.font(14)).padding(.top, 8)
                .accessibilityIdentifier("push-opt-in")
        }
        .accessibilityElement(children: .contain)
    }

    /// The existing per-device opt-in (permission prompt and APNs registration).
    private var pushBinding: Binding<Bool> {
        Binding(get: { model.pushEnabled }, set: { value in Task { await model.changePushEnabled(value) } })
    }

    @ViewBuilder private var status: some View {
        if let error = model.notificationError ?? model.notificationsLoadError {
            Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("notification-settings-error")
        } else if model.notificationSettings == nil {
            Text("Loading…").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
        }
    }
}

private struct NewDirectMessageSheet: View {
    @Bindable var model: AppModel
    let close: () -> Void
    @State private var username = ""
    @State private var pending = false
    @State private var error: String?
    @FocusState private var usernameFocused: Bool
    @State private var suggestions = PeopleSuggestionState()
    private var candidates: [MentionCandidate] { PeopleSuggestions.directMessage(people: model.people, accountID: model.account?.id) }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            SheetHeader(title: "New direct message", detail: "Enter an account’s exact username.", closeDisabled: pending, close: close)
            let choices = suggestions.choices(text: username, candidates: candidates, focused: usernameFocused && !pending)
            VStack(alignment: .leading, spacing: 6) {
                TextField("Exact username", text: Binding(get: { username }, set: { if !pending { username = $0 } })).textFieldStyle(.roundedBorder)
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    #endif
                    .autocorrectionDisabled()
                    .focused($usernameFocused)
                    .onChange(of: username) { _, _ in error = nil }
                    // Return only picks a suggestion; the button starts the conversation.
                    .onSubmit(pickHighlighted)
                    .modifier(PeopleSuggestionKeys(text: username, count: choices.count, state: $suggestions))
                    .accessibilityIdentifier("dm-username")
                if !choices.isEmpty {
                    PeopleSuggestionList(choices: choices, state: suggestions, identifier: "dm-username-suggestions", pick: pick)
                }
            }
            // The frame keeps the error's space; iOS still exposes a transparent
            // placeholder to accessibility, so the text exists only with an error.
            ZStack(alignment: .leading) {
                if let error {
                    Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("dm-create-error")
                }
            }
            .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
            HStack {
                Spacer()
                Button(action: start) {
                    Text("Start conversation").hidden().overlay { Text(pending ? "Opening…" : "Start conversation") }
                }
                // The primary action, as on the other clients: neutral until a username is entered.
                .buttonStyle(CaperPrimaryButton()).frame(width: 180)
                .disabled(pending || username.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityLabel(pending ? "Opening…" : "Start conversation")
                .accessibilityIdentifier("dm-create-submit")
            }
        }.padding(22).onAppear { usernameFocused = true }
            // Suggestions; a refresh keeps the previous list until it lands.
            .task { await model.refreshPeople() }
            .interactiveDismissDisabled(pending).preference(key: DialogDismissDisabled.self, value: pending)
    }
    private func pickHighlighted() {
        let choices = suggestions.choices(text: username, candidates: candidates, focused: !pending)
        if let choice = suggestions.highlightedChoice(in: choices) { pick(choice) }
    }
    /// Fills in the exact username without opening the conversation; the list
    /// stays closed until it is edited.
    private func pick(_ candidate: MentionCandidate) {
        guard !pending else { return }
        username = candidate.username
        suggestions.close(for: username)
    }
    private func start() {
        guard !pending else { return }
        let submitted = username
        pending = true; error = nil
        Task {
            defer { pending = false }
            do {
                if try await model.createDirectMessage(username: submitted) { close() }
                else { error = "Couldn’t open that conversation." }
            } catch { self.error = FriendlyError.message(for: error) }
        }
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
            SheetHeader(title: "Private channel invitation", detail: "#\(invitation.channel.name)", closeDisabled: pending, close: { if !pending { close() } })
            VStack(alignment: .leading, spacing: 14) {
                Text("\(invitation.inviter.displayName) (@\(invitation.inviter.username)) invited you to private channel #\(invitation.channel.name).")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error) }
                HStack {
                    Spacer()
                    Button("Decline") { run { try await model.declineChannelInvitation(invitation) } }.buttonStyle(CaperSecondaryButton(minHeight: 42))
                    // A compact Accept: the primary style otherwise stretches across the dialog.
                    Button("Accept") { run { try await model.acceptChannelInvitation(invitation) } }.buttonStyle(CaperPrimaryButton()).frame(width: 120)
                }.disabled(pending)
            }.padding(22)
        }.background(CaperTheme.surface)
            // Like InvitationSheet: no backdrop or Escape dismissal while the answer is sent.
            .interactiveDismissDisabled(pending).preference(key: DialogDismissDisabled.self, value: pending)
    }
    private func run(_ action: @escaping () async throws -> Void) {
        guard !pending else { return }; pending = true; error = nil
        Task { do { try await action(); close() } catch { self.error = FriendlyError.message(for: error) }; pending = false }
    }
}

private struct SheetHeader: View {
    let title: String; var detail: String?; var closeLabel = "Close"; var titleIcon: Image? = nil
    /// A request is running: closing now could still navigate when it completes.
    var closeDisabled = false
    let close: () -> Void
    var body: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 12) {
                    if let titleIcon { titleIcon.resizable().interpolation(.high).frame(width: 32, height: 32).accessibilityHidden(true) }
                    Text(title).font(CaperTheme.font(20, weight: .bold))
                }
                if let detail { Text(detail).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted) }
            }
            Spacer()
            Button(action: close) { CaperIcon(name: "x") }.buttonStyle(SidebarIconButton()).disabled(closeDisabled).accessibilityLabel(closeLabel)
                .keyboardShortcut(.cancelAction)
                // Centred on the title's line: laid out at that line's height, while
                // the 44pt iPhone target extends evenly into the header's padding.
                .frame(height: titleIcon == nil ? 27 : 32)
        }.padding(22).overlay(alignment: .bottom) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
    }
}

private struct LoginPage: View {
    @Bindable var model: AppModel
    let close: () -> Void
    @State private var email = ""
    @State private var code = ""
    /// A resend succeeded: "We sent a new code…" shows while there is no error.
    @State private var resent = false
    @FocusState private var emailFocused: Bool
    @FocusState private var codeFocused: Bool
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Wordmark().padding(.bottom, 58)
                Text(model.challengeID == nil ? "Welcome to Caper" : "Check your email.")
                    .font(CaperTheme.font(52, weight: .black)).tracking(-2.5).padding(.bottom, 4)
                Text(model.challengeID == nil ? "Use your email to create an account or return to one. We’ll send a code to your email." : "Enter the six-character code sent to \(email.trimmingCharacters(in: .whitespacesAndNewlines)). It expires in 10 minutes.")
                    .font(CaperTheme.font(16)).foregroundStyle(CaperTheme.muted).lineSpacing(7).padding(.bottom, 20)
                if model.challengeID == nil {
                    CaperField(title: "Email address", text: input($email, clearsError: true), placeholder: "you@example.com", focus: $emailFocused)
                    if let error = model.error { LoginError(message: error).padding(.top, 18) }
                    Button { Task { await model.requestCode(email: email) } } label: { actionLabel("Email me a code", busy: "Sending…") }
                        .buttonStyle(LoginActionButton(minWidth: 200)).disabled(model.busy || email.isEmpty)
                        .accessibilityLabel(model.busy ? "Sending…" : "Email me a code")
                        .frame(maxWidth: .infinity, alignment: .trailing).padding(.top, 12)
                } else {
                    // An exhausted code keeps its error; a new code re-enables the field.
                    CaperField(title: "Sign-in code", text: input($code, clearsError: model.loginAttemptsRemaining != 0), focus: $codeFocused)
                        .disabled(model.loginAttemptsRemaining == 0)
                        .onChange(of: code) { _, value in
                            // A keystroke during a request is put back by `input`, not filtered into the code.
                            guard !model.busy else { return }
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
                    if resent && model.error == nil {
                        Text("We sent a new code. Earlier codes no longer work.").font(CaperTheme.font(14)).foregroundStyle(CaperTheme.muted)
                            .fixedSize(horizontal: false, vertical: true).padding(.top, 12)
                            .accessibilityIdentifier("login-code-resent")
                    }
                    if model.loginAttemptsRemaining == 0 {
                        // Also a resend: it counts toward the limit and starts the wait. Past
                        // the limit no code would arrive, so only the note below remains.
                        if model.codesSent <= CodeResend.maximumResends {
                            Button { code = ""; sendNewCode() } label: { actionLabel("Email me a new code", busy: "Sending…") }
                                .buttonStyle(LoginActionButton(minWidth: 232)).disabled(model.busy)
                                .accessibilityLabel(model.busy ? "Sending…" : "Email me a new code")
                                .frame(maxWidth: .infinity, alignment: .trailing).padding(.top, 12)
                        }
                    } else {
                        Button { Task { await model.verify(code: code); if model.account != nil && model.phase != .onboarding { close() } } } label: {
                            actionLabel("Continue", busy: "Checking…")
                        }.buttonStyle(LoginActionButton(minWidth: 200)).disabled(model.busy || code.count != 6)
                            .accessibilityLabel(model.busy ? "Checking…" : "Continue")
                            .frame(maxWidth: .infinity, alignment: .trailing).padding(.top, 12)
                    }
                    // Side by side when they fit; otherwise Resend code goes below.
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .firstTextBaseline, spacing: 24) { differentEmailButton; resendButton }
                        VStack(alignment: .leading, spacing: 14) { differentEmailButton; resendButton }
                    }.padding(.top, 18)
                    if model.codesSent > CodeResend.maximumResends {
                        // In place of Resend code: the server sends no more codes for now.
                        Text("Still nothing? Check your spam folder, or try again in 15 minutes.").foregroundStyle(CaperTheme.muted)
                            .fixedSize(horizontal: false, vertical: true).padding(.top, 14)
                            .accessibilityIdentifier("login-resend-limit")
                    }
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
            // As on web, the current field is focused on each step and when a request ends.
            .onChange(of: model.busy) { _, busy in if !busy { focusField() } }
            .onChange(of: model.challengeID) { _, _ in if !model.busy { focusField() } }
    }

    private func focusField() {
        if model.challengeID == nil { emailFocused = true }
        else if model.loginAttemptsRemaining != 0 { codeFocused = true }
    }

    /// The label, then an arrow; while a request runs, a spinner and the pending label.
    private func actionLabel(_ title: String, busy: String) -> some View {
        LoginActionLabel(title: title, busyTitle: busy, busy: model.busy)
    }

    private var differentEmailButton: some View {
        Button("Use a different email") { resent = false; model.challengeID = nil; model.error = nil }
            .buttonStyle(.plain).foregroundStyle(CaperTheme.muted).modifier(ControlHover()).disabled(model.busy)
    }

    /// Web's Resend code: unavailable for a minute after each code, with an
    /// m:ss countdown, and at most two resends per email entry. Hidden once the
    /// code's attempts are used up, where Email me a new code does the same.
    @ViewBuilder private var resendButton: some View {
        if model.codesSent <= CodeResend.maximumResends && model.loginAttemptsRemaining != 0 {
            TimelineView(.periodic(from: model.codeSentAt ?? .now, by: 1)) { context in
                let remaining = CodeResend.secondsRemaining(sentAt: model.codeSentAt, now: context.date)
                let label = CodeResend.label(secondsRemaining: remaining)
                Button(action: sendNewCode) {
                    // Sized for the countdown, so the row never re-wraps as it ticks.
                    ZStack(alignment: .leading) {
                        Text(CodeResend.label(secondsRemaining: 60)).hidden()
                        Text(label)
                    }.monospacedDigit()
                }
                .buttonStyle(.plain).foregroundStyle(CaperTheme.muted).modifier(ControlHover())
                .disabled(model.busy || remaining > 0)
                .accessibilityLabel(label)
                .accessibilityIdentifier("login-resend-code")
            }
        }
    }

    /// Requests another code for the same email, as the first request did.
    private func sendNewCode() {
        guard !model.busy, model.challengeID != nil, model.codesSent <= CodeResend.maximumResends else { return }
        let sent = model.codesSent
        resent = false
        Task {
            // On success the model switches to the new challenge and resets attempts.
            await model.requestCode(email: email)
            // Errors keep the sign-in mapping; focus returns to the code field when the request ends.
            guard model.codesSent > sent, model.challengeID != nil, model.error == nil else { return }
            code = ""
            resent = true
        }
    }

    /// While a request runs, a field keeps focus but takes no edits, like web's
    /// read-only input: disabling it dropped focus, so a retry needed another
    /// click or tap. A rejected keystroke is written and put back next turn so
    /// the field redraws with the kept text. Other edits clear the last error.
    private func input(_ text: Binding<String>, clearsError: Bool) -> Binding<String> {
        Binding(get: { text.wrappedValue }, set: { value in
            guard value != text.wrappedValue else { return }
            let kept = text.wrappedValue
            text.wrappedValue = value
            if model.busy { DispatchQueue.main.async { text.wrappedValue = kept } }
            else if clearsError, model.error != nil { model.error = nil }
        })
    }
}

private struct LoginError: View {
    let message: String
    var body: some View {
        Text(message).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.text).padding(.horizontal, 14)
            .frame(maxWidth: .infinity, minHeight: 50, alignment: .leading)
            .background(CaperTheme.surface, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.terracottaBright))
    }
}

/// Web's primary action on sign-in and the profile page: compact and
/// right-aligned, at least `minWidth` wide so a pending label never resizes it.
private struct LoginActionButton: ButtonStyle {
    var minWidth: CGFloat? = nil
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(CaperTheme.font(16, weight: .bold))
            .padding(.horizontal, 20).frame(minWidth: minWidth).frame(height: 58)
            .modifier(PrimaryActionFill(pressed: configuration.isPressed))
    }
}

/// A `LoginActionButton` label: the title and an arrow, or a spinner and the
/// pending title while its request runs.
private struct LoginActionLabel: View {
    let title: String
    let busyTitle: String
    let busy: Bool
    var body: some View {
        HStack(spacing: 12) {
            if busy {
                // Busy buttons are disabled, so the spinner sits on neutral chrome.
                ProgressView().controlSize(.small).tint(CaperTheme.muted)
                Text(busyTitle)
            } else {
                Text(title)
                CaperIcon(name: "arrow-right", size: 14)
            }
        }
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
                    Text(model.error ?? " ").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                        .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
                        .opacity(model.error == nil ? 0 : 1).accessibilityHidden(model.error == nil)
                    Button(model.busy ? "Saving…" : "Save profile", action: save)
                        .buttonStyle(CaperPrimaryButton())
                        .frame(width: 148)
                        .frame(maxWidth: .infinity, alignment: .trailing)
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
    /// The answer being sent ("accept" or "decline"); only that button shows progress.
    @State private var pending: String?
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            SheetHeader(title: "You’re invited!", detail: nil, titleIcon: Image("IncomingEnvelope", bundle: artworkBundle), closeDisabled: pending != nil, close: close)
            Text("Join \(invitation.name)?").font(CaperTheme.font(20, weight: .bold)).padding(.horizontal, 22)
            if let inviter = invitation.inviter {
                Text("\(inviter.displayName) (@\(inviter.username)) invited you.").font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted).padding(.horizontal, 22)
            }
            if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error).padding(.horizontal, 22) }
            HStack {
                Spacer()
                Button(pending == "decline" ? "Declining…" : "Decline") { perform("decline") { try await model.declineInvitation(invitation); close() } }.buttonStyle(CaperSecondaryButton(minHeight: 42))
                Button(pending == "accept" ? "Accepting…" : "Accept") { perform("accept") { try await model.acceptInvitation(invitation); close() } }.buttonStyle(CaperPrimaryButton()).frame(width: 120)
            }.disabled(pending != nil).padding(22)
        }.background(CaperTheme.surface)
            .interactiveDismissDisabled(pending != nil).preference(key: DialogDismissDisabled.self, value: pending != nil)
    }

    private var artworkBundle: Bundle {
        #if SWIFT_PACKAGE
        Bundle.module
        #else
        Bundle(for: CaperEffects.self)
        #endif
    }

    private func perform(_ answer: String, _ action: @escaping () async throws -> Void) {
        guard pending == nil else { return }
        pending = answer; error = nil
        Task { do { try await action() } catch { self.error = FriendlyError.message(for: error) }; pending = nil }
    }
}

/// "Join with invite link": paste a link or code, preview the space, then join.
private struct JoinInviteSheet: View {
    @Bindable var model: AppModel
    /// A link opened from another app: filled in and looked up at once.
    var opened: OpenedInviteLink? = nil
    let close: () -> Void
    @State private var link = ""
    @State private var preview: InvitePreview?
    @State private var pending = false
    @State private var error: String?
    @FocusState private var linkFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SheetHeader(title: preview?.alreadyMember == true ? "Open space" : "Join with invite link", closeDisabled: pending, close: close)
            VStack(alignment: .leading, spacing: 16) {
                if let preview { InvitePreviewCard(preview: preview) } else { linkField }
                if let error {
                    Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("invite-link-error")
                }
                actions
            }.padding(22)
        }.background(CaperTheme.surface)
            .onAppear {
                // As if the link had been pasted and Continue pressed.
                if let opened, link.isEmpty { link = opened.link; lookUp() } else { linkFocused = true }
            }
            .interactiveDismissDisabled(pending).preference(key: DialogDismissDisabled.self, value: pending)
    }

    private var linkField: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text("Invite link").font(CaperTheme.font(12, weight: .bold))
            TextField("https://caper.chat/invite/…", text: Binding(get: { link }, set: { if !pending { link = $0; error = nil } }))
                .textFieldStyle(CaperTextFieldStyle())
                .autocorrectionDisabled()
                #if os(iOS)
                .textInputAutocapitalization(.never)
                .keyboardType(.URL)
                #endif
                .focused($linkFocused)
                .submitLabel(.go).onSubmit(lookUp)
                .accessibilityLabel("Invite link")
                .accessibilityIdentifier("invite-link-field")
        }
    }

    private var actions: some View {
        HStack {
            Spacer()
            if preview == nil {
                Button(pending ? "Checking…" : "Continue", action: lookUp).buttonStyle(CaperPrimaryButton()).frame(width: 140)
                    .disabled(pending || link.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .keyboardShortcut(.defaultAction)
                    .accessibilityIdentifier("invite-link-continue")
            } else {
                Button("Cancel", action: close).buttonStyle(CaperSecondaryButton(minHeight: 42)).disabled(pending)
                    .keyboardShortcut(.cancelAction)
                Button(preview?.alreadyMember == true ? (pending ? "Opening…" : "Open space") : (pending ? "Joining…" : "Join space"), action: join).buttonStyle(CaperPrimaryButton()).frame(width: 140)
                    .disabled(pending).keyboardShortcut(.defaultAction)
                    .accessibilityIdentifier("invite-link-join")
            }
        }
    }

    private func lookUp() {
        guard !pending, preview == nil else { return }
        guard let code = InviteLinks.code(from: link) else { error = InviteLinks.invalid; return }
        pending = true; error = nil
        Task {
            do { preview = try await model.invitePreview(code: code) }
            catch { self.error = Self.message(for: error) }
            pending = false
        }
    }

    private func join() {
        guard !pending, let code = preview?.code else { return }
        pending = true; error = nil
        Task {
            do {
                try await model.joinInvite(code: code)
                pending = false
                close()
            } catch {
                self.error = Self.message(for: error)
                pending = false
            }
        }
    }

    /// A dead link (404) reads the same however it died; refusals (403/409)
    /// show the server's text as a sentence.
    static func message(for error: Error) -> String {
        if let failure = error as? APIError, failure.status == 404 { return InviteLinks.invalid }
        return FriendlyError.message(for: error)
    }
}

/// The space an invite link opens: name, inviter, size and landing channel.
private struct InvitePreviewCard: View {
    let preview: InvitePreview
    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            Text(String(preview.space.name.prefix(1)).uppercased()).font(CaperTheme.font(18, weight: .black))
                .frame(width: 52, height: 52).background(CaperTheme.raised)
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(CaperTheme.border))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 6) {
                Text(preview.space.name).font(CaperTheme.font(18, weight: .bold)).foregroundStyle(CaperTheme.text)
                    .fixedSize(horizontal: false, vertical: true)
                if preview.alreadyMember == true { Text("You’re already a member") }
                HStack(spacing: 6) {
                    Avatar(name: preview.inviter.displayName, size: 18, avatarID: preview.inviter.avatarId)
                    Text("Invited by \(preview.inviter.displayName)").lineLimit(1)
                }
                Text(InviteLinks.memberCount(preview.memberCount))
                if let channel = preview.channel { Text("Opens in #\(channel.name)").lineLimit(1) }
            }.font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
            Spacer(minLength: 0)
        }
        .padding(14).frame(maxWidth: .infinity, alignment: .leading)
        .background(CaperTheme.composer).clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("invite-preview")
    }
}

/// Manage space's "Invite link": owners create, copy and revoke shareable links.
private struct InviteLinksSection: View {
    @Bindable var model: AppModel
    @State private var expiryNumber = InviteLinks.defaultExpiryNumber
    @State private var expiryUnit = InviteLinks.defaultExpiryUnit
    /// Empty is No limit.
    @State private var maxUses = ""
    @State private var expiryError: String?
    @State private var maxUsesError: String?
    @State private var channelID: String?
    @State private var links: [InviteLink] = []
    @State private var loading = false
    /// "create", or the code of the link being revoked.
    @State private var pending: String?
    @State private var error: String?
    /// The link whose Copy button reads "Copied".
    @State private var copied: String?
    @State private var copiedReset: Task<Void, Never>?

    private var publicChannels: [Channel] { (model.detail?.channels ?? []).filter { !$0.private } }
    private var unitOptions: [InviteOption<InviteExpiryUnit>] {
        InviteExpiryUnit.allCases.map { InviteOption(label: $0.label, value: $0) }
    }
    private var channelOptions: [InviteOption<String?>] {
        [InviteOption<String?>(label: "Default channel", value: nil)]
            + publicChannels.map { InviteOption<String?>(label: "#\($0.name)", value: $0.id) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Invite link").font(CaperTheme.font(14, weight: .bold))
                Text("Anyone with the link can join this space.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
            }
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .top, spacing: 10) { pickers }
                VStack(alignment: .leading, spacing: 10) { pickers }
            }
            Button(pending == "create" ? "Creating…" : "Create link", action: create)
                .buttonStyle(CaperSecondaryButton(minHeight: 42)).disabled(pending != nil)
                .accessibilityIdentifier("invite-link-create")
            if let error {
                Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if loading && links.isEmpty { ProgressView("Loading invite links…").font(CaperTheme.font(12)) }
            ForEach(links) { link in row(link) }
        }
        .onAppear {
            // Opens in the channel being viewed when anyone can see it.
            if let channel = model.selectedChannel, !channel.private { channelID = channel.id }
        }
        .task(id: model.detail?.space.id) { await load() }
        .onDisappear { copiedReset?.cancel() }
    }

    @ViewBuilder private var pickers: some View {
        expiryField
        maxUsesField
        InviteOptionPicker(title: "Opens in", selection: $channelID, options: channelOptions)
    }

    /// A whole number and its unit; Never hides the number.
    private var expiryField: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text("Expire after").font(CaperTheme.font(12, weight: .bold))
            HStack(spacing: 8) {
                if expiryUnit != .never {
                    numberField("7", text: Binding(get: { expiryNumber }, set: { expiryNumber = $0; expiryError = nil }))
                        .frame(width: 76)
                        .accessibilityLabel("Expire after")
                        .accessibilityIdentifier("invite-link-expiry-number")
                }
                InviteOptionPicker(title: "Expiry unit", selection: Binding(get: { expiryUnit }, set: { expiryUnit = $0; expiryError = nil }),
                                   options: unitOptions, showsTitle: false)
                    .accessibilityIdentifier("invite-link-expiry-unit")
            }
            if let expiryError { fieldError(expiryError).accessibilityIdentifier("invite-link-expiry-error") }
        }
    }

    private var maxUsesField: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text("Max uses").font(CaperTheme.font(12, weight: .bold))
            numberField("No limit", text: Binding(get: { maxUses }, set: { maxUses = $0; maxUsesError = nil }))
                .frame(width: 140)
                .accessibilityLabel("Max uses")
                .accessibilityIdentifier("invite-link-max-uses")
            if let maxUsesError { fieldError(maxUsesError).accessibilityIdentifier("invite-link-max-uses-error") }
        }
    }

    private func numberField(_ placeholder: String, text: Binding<String>) -> some View {
        TextField(placeholder, text: text)
            .textFieldStyle(CaperTextFieldStyle())
            .autocorrectionDisabled()
            #if os(iOS)
            .keyboardType(.numberPad)
            #endif
    }

    private func fieldError(_ message: String) -> some View {
        Text(message).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
            .fixedSize(horizontal: false, vertical: true)
    }

    private func row(_ link: InviteLink) -> some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 3) {
                Text(model.inviteLinkURL(link.code)).font(.system(size: 12, design: .monospaced))
                    .lineLimit(1).truncationMode(.middle).textSelection(.enabled)
                TimelineView(.everyMinute) { context in
                    Text(InviteLinks.meta(for: link, now: context.date)).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                }
            }
            Spacer(minLength: 8)
            Button(copied == link.code ? "Copied" : "Copy") { copy(link) }.buttonStyle(CaperSecondaryButton())
                .accessibilityLabel(copied == link.code ? "Copied" : "Copy invite link")
            Button(pending == link.code ? "Revoking…" : "Revoke", role: .destructive) { revoke(link) }
                .buttonStyle(CaperSecondaryButton()).disabled(pending != nil)
        }
        .padding(10).background(CaperTheme.composer).clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .accessibilityIdentifier("invite-link-\(link.code)")
    }

    private func load() async {
        loading = true; error = nil
        do { links = try await model.loadInviteLinks() }
        catch is CancellationError {}
        catch { self.error = FriendlyError.message(for: error) }
        loading = false
    }

    private func create() {
        guard pending == nil else { return }
        // Out-of-range options show beside their fields and aren't sent.
        let expiry = InviteLinks.expirySeconds(expiryNumber, unit: expiryUnit)
        let uses = InviteLinks.maxUses(maxUses)
        if case .failure(let failure) = expiry { expiryError = failure.message } else { expiryError = nil }
        if case .failure(let failure) = uses { maxUsesError = failure.message } else { maxUsesError = nil }
        guard case .success(let maxAge) = expiry, case .success(let useLimit) = uses else { return }
        // A channel made private since it was picked falls back to the default.
        let channel = channelID.flatMap { id in publicChannels.contains { $0.id == id } ? id : nil }
        pending = "create"; error = nil
        Task {
            do {
                let link = try await model.createInviteLink(channelID: channel, maxAge: maxAge, maxUses: useLimit)
                links.removeAll { $0.code == link.code }
                links.insert(link, at: 0)
                copy(link)
            } catch { self.error = FriendlyError.message(for: error) }
            pending = nil
        }
    }

    private func revoke(_ link: InviteLink) {
        guard pending == nil else { return }
        pending = link.code; error = nil
        Task {
            do {
                try await model.revokeInviteLink(link)
                links.removeAll { $0.code == link.code }
            } catch let failure as APIError where failure.status == 404 {
                // Already expired, used up or revoked elsewhere.
                links.removeAll { $0.code == link.code }
            } catch { self.error = FriendlyError.message(for: error) }
            pending = nil
        }
    }

    private func copy(_ link: InviteLink) {
        let url = model.inviteLinkURL(link.code)
        #if os(macOS)
        NSPasteboard.general.clearContents()
        guard NSPasteboard.general.setString(url, forType: .string) else { error = "Copy failed; try again."; return }
        #else
        UIPasteboard.general.string = url
        #endif
        copied = link.code
        copiedReset?.cancel()
        copiedReset = Task {
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            copied = nil
        }
    }
}

/// A labelled menu of fixed choices, styled like the audio device pickers.
private struct InviteOptionPicker<Value: Hashable & Sendable>: View {
    let title: String
    @Binding var selection: Value
    let options: [InviteOption<Value>]
    /// False when a surrounding label already names the menu; `title` stays its accessibility label.
    var showsTitle = true
    @Environment(\.isEnabled) private var isEnabled
    private var current: String { options.first { $0.value == selection }?.label ?? options.first?.label ?? "" }
    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            if showsTitle { Text(title).font(CaperTheme.font(12, weight: .bold)) }
            Menu {
                ForEach(options, id: \.label) { option in
                    Button(option.label) { selection = option.value }
                }
            } label: {
                HStack(spacing: 8) {
                    Text(current).lineLimit(1)
                    Spacer(minLength: 0)
                    CaperIcon(name: "chevron-down", size: 12)
                }.font(CaperTheme.font(12)).foregroundStyle(CaperTheme.text)
                    .padding(.horizontal, 11).frame(minWidth: 140, minHeight: 42)
                    .background(CaperTheme.composer)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                    .opacity(isEnabled ? 1 : 0.45)
            }.menuStyle(.borderlessButton).menuIndicator(.hidden)
                .accessibilityLabel(title).accessibilityValue(current)
                .modifier(ControlHover())
        }
    }
}

private struct SpaceEditor: View {
    @Bindable var model: AppModel; let close: () -> Void; let managing: Bool
    /// Create a space only: switches to "Join with invite link".
    var joinInvite: (() -> Void)? = nil
    @State private var name = ""; @State private var username = ""; @State private var error: String?; @State private var pending = false
    @State private var confirmDelete = false
    /// The member whose removal is being confirmed.
    @State private var removing: Member?
    @FocusState private var nameFocused: Bool
    @FocusState private var memberFocused: Bool
    /// An invite finished: return focus to the field so several people can be invited in a row.
    @State private var refocusMember = false
    /// Creating can still navigate when it completes, so the sheet stays until then.
    private var creating: Bool { !managing && pending }
    /// Web's member form: its confirmation or its error, under the field.
    @State private var memberStatus: String?
    @State private var memberError: String?
    @State private var suggestions = PeopleSuggestionState()
    /// People you know who are neither in the space nor invited to it.
    private var inviteCandidates: [MentionCandidate] {
        PeopleSuggestions.spaceInvite(people: model.people, members: model.detail?.members ?? [],
                                      invited: model.pendingMembers, accountID: model.account?.id)
    }
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: managing ? "Manage space" : "Create a space", detail: managing ? "Only the owner can change this space and its membership." : nil, closeDisabled: creating, close: close)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    CaperField(title: "Space name", text: $name, focus: $nameFocused)
                        .onChange(of: name) { _, _ in error = nil }
                    Button(pending ? (managing ? "Saving…" : "Creating…") : managing ? "Save name" : "Create space") { run { if managing { try await model.renameSpace(name) } else { try await model.createSpace(name: name); close() } } }.buttonStyle(CaperPrimaryButton())
                        .disabled(pending || (!managing && model.limits != nil && !model.canCreateSpace))
                    if !managing { createFooter }
                    if managing {
                        Divider().overlay(CaperTheme.border)
                        InviteLinksSection(model: model)
                        Divider().overlay(CaperTheme.border)
                        Text("Members  \(model.detail?.members.count ?? 0)").font(CaperTheme.font(14, weight: .bold))
                            .accessibilityLabel("Members \(model.detail?.members.count ?? 0)")
                            .accessibilityIdentifier("space-members-heading")
                        let choices = suggestions.choices(text: username, candidates: inviteCandidates, focused: memberFocused)
                        VStack(alignment: .leading, spacing: 6) {
                            // The field stays enabled during a request so it keeps focus; addMember ignores it until then.
                            HStack {
                                TextField("Exact username", text: usernameField).textFieldStyle(CaperTextFieldStyle())
                                    .autocorrectionDisabled()
                                    #if os(iOS)
                                    .textInputAutocapitalization(.never)
                                    #endif
                                    .focused($memberFocused)
                                    .submitLabel(.done).onSubmit(submitMember)
                                    .modifier(PeopleSuggestionKeys(text: username, count: choices.count, state: $suggestions))
                                    .accessibilityLabel("Exact username")
                                Button("Invite", action: addMember).buttonStyle(CaperSecondaryButton(minHeight: 42))
                                    .disabled(pending || WorkspaceValidation.usernameError(username) != nil)
                            }
                            if !choices.isEmpty {
                                PeopleSuggestionList(choices: choices, state: suggestions, identifier: "space-invite-suggestions", pick: pickMember)
                            }
                        }
                        if let memberStatus { Text(memberStatus).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted) }
                        if let memberError { Text(memberError).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error) }
                        ForEach(model.detail?.members ?? []) { member in
                            HStack { Avatar(name: member.displayName, size: 30, avatarID: member.avatarId); VStack(alignment: .leading) { Text(member.displayName); Text("@\(member.username)\(member.owner ? " · Owner" : "")").foregroundStyle(CaperTheme.muted) }; Spacer(); if !member.owner { Button("Remove") { removing = member }.disabled(pending) } }.font(CaperTheme.font(12))
                        }
                        Text("Pending invitations  \(model.pendingMembers.count)").font(CaperTheme.font(14, weight: .bold))
                        ForEach(model.pendingMembers) { member in
                            HStack { Avatar(name: member.displayName, size: 30, avatarID: member.avatarId); VStack(alignment: .leading) { Text(member.displayName); Text("@\(member.username)").foregroundStyle(CaperTheme.muted) }; Spacer(); Button("Cancel") { run { try await model.cancelSpaceInvitation(member) } } }.font(CaperTheme.font(12))
                        }
                        Divider().overlay(CaperTheme.border)
                        Text("Delete space").font(CaperTheme.font(14, weight: .bold))
                        Text("Delete this space and all its channels for every member.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                        Button("Delete space", role: .destructive) { CaperEffects.shared.play(.warning); confirmDelete = true }.buttonStyle(CaperSecondaryButton()).disabled(pending)
                    }
                    if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error) }
                }.padding(22)
            }.scrollDismissesKeyboard(.interactively).accessibilityIdentifier("space-settings-scroll")
        }.background(CaperTheme.surface).onAppear { name = managing ? model.detail?.space.name ?? "" : ""; if !managing { nameFocused = true } }
        .onChange(of: pending) { _, running in if !running && refocusMember { refocusMember = false; memberFocused = true } }
        .interactiveDismissDisabled(creating).preference(key: DialogDismissDisabled.self, value: creating)
        .task(id: model.detail?.space.id) { if managing { do { try await model.loadSpaceInvitations() } catch { self.error = FriendlyError.message(for: error) } } }
        // Invite suggestions; a refresh keeps the previous list until it lands.
        .task { if managing { await model.refreshPeople() } }
        .sheet(isPresented: $confirmDelete) {
            ConfirmationSheet(title: "Delete space", detail: "Delete \(model.detail?.space.name ?? name) for everyone? All its channels and their messages will disappear from the space. This cannot be undone.", action: "Delete space", close: { confirmDelete = false }) {
                try await model.deleteCurrentSpace()
                CaperEffects.shared.play(.delete)
                close()
            }
            .modifier(ConfirmationPresentation())
        }
        // Removing a member asks first; the dialog stays open with them gone.
        .sheet(item: $removing) { member in
            ConfirmationSheet(title: "Remove \(member.displayName)?", detail: "They’ll lose access to \(model.detail?.space.name ?? name) and its channels. You can invite them again later.", action: "Remove", pendingLabel: "Removing…", close: { removing = nil }) {
                try await model.removeSpaceMember(member)
            }
            .modifier(ConfirmationPresentation())
        }
    }
    /// Create a space: the space limit, and joining by invite link instead.
    @ViewBuilder private var createFooter: some View {
        if model.limits != nil && !model.canCreateSpace {
            Text("You have reached your space limit.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
        }
        if let joinInvite {
            Divider().overlay(CaperTheme.border)
            HStack {
                Text("Have an invite link?").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                Spacer()
                Button("Join with invite link", action: joinInvite).buttonStyle(CaperSecondaryButton(minHeight: 42))
                    .disabled(pending).accessibilityIdentifier("join-invite-link")
            }
        }
    }
    /// Typing another username clears the last result, as on web.
    private var usernameField: Binding<String> {
        Binding(get: { username }, set: { value in
            let next = WorkspaceValidation.normalizeUsername(value)
            if next != username { memberStatus = nil; memberError = nil }
            username = next
        })
    }
    /// Return picks the highlighted suggestion while the list is open; it invites otherwise.
    private func submitMember() {
        let choices = suggestions.choices(text: username, candidates: inviteCandidates, focused: true)
        if let choice = suggestions.highlightedChoice(in: choices) { pickMember(choice) } else { addMember() }
    }
    /// Fills in the exact username without inviting; the list stays closed until it is edited.
    private func pickMember(_ candidate: MentionCandidate) {
        usernameField.wrappedValue = candidate.username
        suggestions.close(for: username)
    }
    private func addMember() {
        guard !pending, WorkspaceValidation.usernameError(username) == nil else { return }
        let submitted = username
        memberStatus = nil; memberError = nil
        if let duplicate = WorkspaceValidation.inviteError(username: submitted, members: model.detail?.members ?? [],
                                                           invited: model.pendingMembers, channel: false) {
            memberError = duplicate; return
        }
        // A failure keeps the typed username, and so does anything typed while the invite was sent.
        pending = true; error = nil; refocusMember = true
        Task {
            do { try await model.addSpaceMember(username: submitted); if username == submitted { username = "" }; memberStatus = WorkspaceValidation.invitationSent }
            catch { memberError = WorkspaceValidation.memberMessage(error) }
            pending = false
        }
    }
    private func run(_ action: @escaping () async throws -> Void) { guard !pending else { return }; pending = true; error = nil; Task { do { try await action() } catch { self.error = FriendlyError.message(for: error) }; pending = false } }
}

private struct ChannelEditor: View {
    @Bindable var model: AppModel; @State var channel: Channel?; let close: () -> Void
    @State private var name = ""; @State private var privateChannel = false; @State private var members: [Member] = []; @State private var username = ""; @State private var error: String?; @State private var pending = false
    @State private var confirmDelete = false
    /// The member whose removal is being confirmed.
    @State private var removing: Member?
    @State private var membersError: String?
    @State private var invitations: [Member] = []
    @State private var memberError: String?
    @State private var memberStatus: String?
    @State private var loadingMembers = false
    @FocusState private var nameFocused: Bool
    @FocusState private var memberFocused: Bool
    /// An invite finished: return focus to the field so several people can be invited in a row.
    @State private var refocusMember = false
    @State private var suggestions = PeopleSuggestionState()
    /// Space members without access to this channel or an invitation to it;
    /// none until both lists have loaded.
    private var grantCandidates: [MentionCandidate] {
        guard !loadingMembers, membersError == nil else { return [] }
        return PeopleSuggestions.channelGrant(spaceMembers: model.detail?.members ?? [], channelMembers: members,
                                              invited: invitations, accountID: model.account?.id)
    }
    private var dirty: Bool { channel.map { name != $0.name || privateChannel != $0.private } ?? false }
    /// Creating can still navigate when it completes, so the sheet stays until then.
    private var creating: Bool { channel == nil && pending }
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: channel == nil ? "Create a channel" : "Channel settings", closeDisabled: creating, close: close)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    VStack(alignment: .leading, spacing: 7) {
                        Text("Channel name").font(CaperTheme.font(12, weight: .bold))
                        HStack(spacing: 8) {
                            if channel == nil { CaperIcon(name: privateChannel ? "lock" : "hash", size: 16).foregroundStyle(CaperTheme.muted) }
                            TextField("project-updates", text: Binding(get: { name }, set: { name = WorkspaceValidation.normalizeChannelName($0) }))
                                .focused($nameFocused).onSubmit(submit)
                                .onChange(of: name) { _, _ in error = nil }
                        }.textFieldStyle(CaperTextFieldStyle())
                    }
                    if channel == nil {
                        Text("Channels are where conversations happen around a topic. Use a name that is easy to find and understand.")
                            .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).fixedSize(horizontal: false, vertical: true)
                    }
                    Toggle(isOn: Binding(get: { privateChannel }, set: { privateChannel = $0; CaperEffects.shared.toggle($0) })) { VStack(alignment: .leading) { Text("Private channel").font(CaperTheme.font(13, weight: .bold)); Text(privateChannel ? "Only you and the people you add can view or join." : "Anyone in \(model.detail?.space.name ?? "this space") can view or join this channel.").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted) }.frame(minHeight: channel == nil ? nil : 32, alignment: .topLeading) }.toggleStyle(.switch)
                    if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error) }
                    if channel == nil {
                        HStack {
                            Spacer()
                            Button("Cancel", action: close).buttonStyle(CaperSecondaryButton(minHeight: 42)).disabled(pending).keyboardShortcut(.cancelAction)
                            Button(pending ? "Creating…" : "Create channel", action: submit).buttonStyle(CaperPrimaryButton()).frame(width: 160)
                                .disabled(pending || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty).keyboardShortcut(.defaultAction)
                        }
                    }
                    if let channel, channel.private {
                        Divider().overlay(CaperTheme.border)
                        HStack { Text("Members").font(CaperTheme.font(14, weight: .bold)); Text("\(members.count)").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted) }
                        if loadingMembers { ProgressView("Loading members…") }
                        if let membersError {
                            Text(membersError).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                            Button("Retry loading members") { Task { await loadMembers(channel) } }.disabled(loadingMembers)
                        }
                        let choices = suggestions.choices(text: username, candidates: grantCandidates, focused: memberFocused)
                        VStack(alignment: .leading, spacing: 6) {
                            // Not disabled while an invite is sent, so the field keeps focus; addMember ignores it until then.
                            HStack {
                                TextField("Exact username", text: usernameField).textFieldStyle(CaperTextFieldStyle())
                                    .autocorrectionDisabled()
                                    #if os(iOS)
                                    .textInputAutocapitalization(.never)
                                    #endif
                                    .focused($memberFocused)
                                    .submitLabel(.done).onSubmit { submitMember(channel) }
                                    .modifier(PeopleSuggestionKeys(text: username, count: choices.count, state: $suggestions))
                                    .accessibilityLabel("Exact username")
                                Button("Invite") { addMember(channel) }.buttonStyle(CaperSecondaryButton(minHeight: 42))
                                    .disabled(pending || WorkspaceValidation.usernameError(username) != nil)
                            }
                                .disabled(loadingMembers || membersError != nil)
                            if !choices.isEmpty {
                                PeopleSuggestionList(choices: choices, state: suggestions, identifier: "channel-member-suggestions", pick: pickMember)
                            }
                        }
                        if let memberStatus { Text(memberStatus).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted) }
                        if let memberError { Text(memberError).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error) }
                        ForEach(members) { member in
                            HStack {
                                Avatar(name: member.displayName, size: 30, avatarID: member.avatarId)
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(member.displayName).font(CaperTheme.font(12, weight: .bold))
                                    Text("@\(member.username)\(member.owner ? " · Owner" : "")").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                                }
                                Spacer()
                                if !member.owner { Button("Remove") { removing = member } }
                            }.font(CaperTheme.font(12))
                        }.disabled(pending || loadingMembers || membersError != nil)
                        Text("Pending invitations  \(invitations.count)").font(CaperTheme.font(14, weight: .bold))
                        ForEach(invitations) { member in
                            HStack {
                                Avatar(name: member.displayName, size: 30, avatarID: member.avatarId)
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(member.displayName).font(CaperTheme.font(12, weight: .bold))
                                    Text("@\(member.username)").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                                }
                                Spacer()
                                Button("Cancel") { run { try await model.removeChannelMember(channel, member: member); invitations.removeAll { $0.id == member.id } } }
                            }.font(CaperTheme.font(12))
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
            }.accessibilityIdentifier("channel-settings-scroll")
            if channel != nil {
                // Reserve the save bar while clean so toggling privacy cannot resize the sheet.
                let saveBar = HStack {
                    Text("You have unsaved changes.").font(CaperTheme.font(12))
                    Spacer()
                    Button("Reset") { name = channel?.name ?? ""; privateChannel = channel?.private ?? false; error = nil }.buttonStyle(CaperSecondaryButton(minHeight: 42)).disabled(pending)
                    Button(pending ? "Saving…" : "Save changes", action: submit).buttonStyle(CaperPrimaryButton()).frame(width: 150).disabled(pending)
                }.padding(.horizontal, 22).padding(.vertical, 12).background(CaperTheme.raised)
                    .accessibilityIdentifier("channel-save-bar")
                if dirty { saveBar } else { saveBar.hidden().allowsHitTesting(false) }
            }
        }.background(CaperTheme.surface).onAppear { name = channel?.name ?? ""; privateChannel = channel?.private ?? false; if channel == nil { nameFocused = true } }
        .onChange(of: pending) { _, running in if !running && refocusMember { refocusMember = false; memberFocused = true } }
        .interactiveDismissDisabled(creating).preference(key: DialogDismissDisabled.self, value: creating)
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
        // Removing a member asks first; the overview stays open with them gone.
        .sheet(item: $removing) { member in
            if let channel {
                ConfirmationSheet(title: "Remove \(member.displayName) from #\(channel.name)?", detail: "They’ll lose access to this private channel. You can add them again later.", action: "Remove", pendingLabel: "Removing…", close: { removing = nil }) {
                    try await model.removeChannelMember(channel, member: member)
                    members.removeAll { $0.id == member.id }
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
            // Return does nothing until there is a name, as the disabled Create channel button.
            guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
            run {
                let created = try await model.createChannel(name: name, privateChannel: privateChannel)
                // Web opens a new private channel's overview so people can be added.
                if let created, created.private { channel = created; name = created.name; privateChannel = created.private } else { close() }
            }
        }
    }
    /// Typing another username clears the last result, as on web.
    private var usernameField: Binding<String> {
        Binding(get: { username }, set: { value in
            let next = WorkspaceValidation.normalizeUsername(value)
            if next != username { memberStatus = nil; memberError = nil }
            username = next
        })
    }
    /// Return picks the highlighted suggestion while the list is open; it invites otherwise.
    private func submitMember(_ channel: Channel) {
        let choices = suggestions.choices(text: username, candidates: grantCandidates, focused: true)
        if let choice = suggestions.highlightedChoice(in: choices) { pickMember(choice) } else { addMember(channel) }
    }
    /// Fills in the exact username without inviting; the list stays closed until it is edited.
    private func pickMember(_ candidate: MentionCandidate) {
        usernameField.wrappedValue = candidate.username
        suggestions.close(for: username)
    }
    private func addMember(_ channel: Channel) {
        guard !pending, !loadingMembers, membersError == nil else { return }
        guard !username.isEmpty else { memberError = "Enter an exact username."; return }
        let submitted = username
        memberStatus = nil; memberError = nil
        if let duplicate = WorkspaceValidation.inviteError(username: submitted, members: members, invited: invitations, channel: true) {
            memberError = duplicate; return
        }
        // A failure keeps the typed username, and so does anything typed while the invite was sent.
        pending = true; error = nil; refocusMember = true
        Task {
            do {
                let member = try await model.addChannelMember(channel, username: submitted)
                invitations.removeAll { $0.id == member.id }; invitations.append(member)
                if username == submitted { username = "" }
                memberStatus = WorkspaceValidation.invitationSent
            } catch { memberError = WorkspaceValidation.memberMessage(error) }
            pending = false
        }
    }
    private func loadMembers(_ channel: Channel) async {
        guard !loadingMembers else { return }
        loadingMembers = true; membersError = nil
        defer { loadingMembers = false }
        do { members = try await model.channelMembers(channel); invitations = try await model.channelInvitations(channel) }
        catch { membersError = FriendlyError.message(for: error) }
    }
    private func run(_ action: @escaping () async throws -> Void) { pending = true; error = nil; Task { do { try await action() } catch { self.error = FriendlyError.message(for: error) }; pending = false } }
}

private struct DialogDismissDisabled: PreferenceKey {
    static let defaultValue = false
    static func reduce(value: inout Bool, nextValue: () -> Bool) { value = value || nextValue() }
}

private struct ConfirmationSheet: View {
    let title: String; let detail: String; let action: String
    /// The action's label while it runs, e.g. "Leaving…"; deletes default to "Deleting…".
    var pendingLabel: String? = nil
    let close: () -> Void; let perform: () async throws -> Void
    @State private var pending = false; @State private var error: String?
    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: title, detail: detail, closeDisabled: pending, close: { if !pending { close() } })
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let error { Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error).fixedSize(horizontal: false, vertical: true) }
                    // Trailing, like the other dialogs' actions.
                    HStack {
                        Spacer()
                        Button("Cancel", action: close).disabled(pending).keyboardShortcut(.cancelAction)
                        Button(pending ? (pendingLabel ?? (action.hasPrefix("Delete") ? "Deleting…" : "Saving…")) : action, role: .destructive) {
                            guard !pending else { return }
                            pending = true; error = nil
                            Task { do { try await perform(); close() } catch { self.error = FriendlyError.message(for: error) }; pending = false }
                        }.disabled(pending).accessibilityIdentifier("confirm-destructive-action")
                    }
                }.padding(22)
            }
        }.background(CaperTheme.surface).interactiveDismissDisabled(pending).preference(key: DialogDismissDisabled.self, value: pending)
    }
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

struct CaperPrimaryButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View { configuration.label.font(CaperTheme.font(13, weight: .bold)).frame(maxWidth: .infinity).frame(height: 42).modifier(PrimaryActionFill(pressed: configuration.isPressed)) }
}

/// Web's primary action: an eggshell fill with a blackout label that brightens on hover and
/// deepens on press. A white hover wash would vanish on eggshell, so focus is a terracotta ring.
/// Disabled, it steps back to neutral chrome rather than a faded fill.
private struct PrimaryActionFill: ViewModifier {
    let pressed: Bool
    @Environment(\.isEnabled) private var enabled
    @Environment(\.isFocused) private var focused
    @State private var hovered = false
    func body(content: Content) -> some View {
        content.foregroundStyle(enabled ? CaperTheme.onAction : CaperTheme.muted)
            .background(
                !enabled ? CaperTheme.raised : pressed ? CaperTheme.actionPress : hovered ? CaperTheme.actionHover : CaperTheme.action,
                ignoresSafeAreaEdges: []
            )
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay {
                RoundedRectangle(cornerRadius: 8)
                    .strokeBorder(!enabled ? CaperTheme.border : focused ? CaperTheme.terracottaBright : .clear, lineWidth: focused && enabled ? 2 : 1)
                    .allowsHitTesting(false)
            }
            .focusEffectDisabled()
            .onHover { hovered = $0 }
            .modifier(ControlPointer())
    }
}

/// Profile entry points keep their content's size and use the common control feedback.
private struct ProfileButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.foregroundStyle(CaperTheme.text)
            .contentShape(Rectangle())
            .background(configuration.isPressed ? CaperTheme.border.opacity(0.55) : Color.clear)
            .clipShape(RoundedRectangle(cornerRadius: 4))
            .modifier(ControlHover())
    }
}

/// Web's control feedback: a --surface-hover (white 6%) wash under the pointer
/// and a 2pt terracotta ring for keyboard focus, in place of the system's.
private struct ControlHover: ViewModifier {
    @Environment(\.isEnabled) private var enabled
    @Environment(\.isFocused) private var focused
    @State private var hovered = false
    // Modifiers outside a button see its focusable ancestor (e.g. the whole
    // timeline), not that button. Those controls bind their own focus instead.
    var isFocused: Bool? = nil
    /// False where a containing row draws the hover wash across all its controls.
    var wash = true
    private var ringed: Bool { (isFocused ?? focused) && enabled }
    func body(content: Content) -> some View {
        content.overlay {
            RoundedRectangle(cornerRadius: 8)
                .fill(Color.white.opacity(wash && hovered && enabled ? 0.06 : 0))
                .allowsHitTesting(false)
        }
        .overlay {
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(CaperTheme.terracotta, lineWidth: 2)
                .opacity(ringed ? 1 : 0)
                .allowsHitTesting(false)
        }
        .focusEffectDisabled()
        .onHover { hovered = $0 }
        .modifier(ControlPointer())
    }
}

/// AppKit owns the cursor throughout the control's rect, including after SwiftUI redraws.
struct ControlPointer: ViewModifier {
    @Environment(\.isEnabled) private var enabled
    func body(content: Content) -> some View {
        #if os(macOS)
        content.background(PointerRegion(enabled: enabled).allowsHitTesting(false))
        #else
        content
        #endif
    }
}

/// A resize handle's left-right cursor, owned by AppKit like `ControlPointer`'s.
/// Pushing it on hover left it stuck when the handle went away under the
/// pointer (Escape closing the thread, a window narrowing past the sidebar).
private struct ResizeCursor: ViewModifier {
    func body(content: Content) -> some View {
        #if os(macOS)
        content.background(PointerRegion(enabled: true, cursor: .resizeLeftRight).allowsHitTesting(false))
        #else
        content
        #endif
    }
}

#if os(macOS)
private struct PointerRegion: NSViewRepresentable {
    let enabled: Bool
    var cursor: NSCursor = .pointingHand
    func makeNSView(context: Context) -> PointerView { PointerView() }
    func updateNSView(_ view: PointerView, context: Context) {
        view.enabled = enabled
        view.cursor = cursor
        view.window?.invalidateCursorRects(for: view)
    }
    final class PointerView: NSView {
        var enabled = true
        var cursor: NSCursor = .pointingHand
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func resetCursorRects() {
            super.resetCursorRects()
            if enabled { addCursorRect(visibleRect, cursor: cursor) }
        }
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            window?.invalidateCursorRects(for: self)
        }
    }
}
#endif

struct CaperSecondaryButton: ButtonStyle {
    /// Beside a primary button or a field (42pt), match their height.
    var minHeight: CGFloat? = nil
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(CaperTheme.font(12, weight: .medium))
            .foregroundStyle(configuration.role == .destructive ? CaperTheme.terracottaBright : CaperTheme.text)
            .padding(.horizontal, 12).padding(.vertical, 8)
            .frame(minHeight: minHeight)
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
    @Environment(\.isEnabled) private var isEnabled
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
                // Dimmed like the other controls while it can't be changed (during a microphone test).
                .opacity(isEnabled ? 1 : 0.45)
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
                // The dialogs' shared close button: 44pt on iPhone, with a pressed state on macOS.
                Button(action: close) { CaperIcon(name: "x") }
                .buttonStyle(SidebarIconButton())
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
            if let routeError { Text(routeError).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error) }
            #else
            AudioRouteRow(title: "Microphone", value: voice.availableInputs.first(where: { $0.id == voice.selectedInputID })?.name ?? "System default")
            #endif
            HStack { Text("Microphone volume"); Spacer(); Text("\(voice.inputGain)%") }.font(CaperTheme.font(12)).monospacedDigit()
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
            HStack { Text("Speaker volume"); Spacer(); Text("\(voice.outputGain)%") }.font(CaperTheme.font(12)).monospacedDigit()
            CaperSlider(value: outputGain, in: 0...200, step: 1)
                .accessibilityLabel("Test speaker volume")
                .accessibilityValue("\(voice.outputGain)%")
            Button(speakerTest.playing ? "Stop speaker test" : "Test speakers") { speakerTest.toggle(voice: voice) }
                .buttonStyle(VoiceJoinButton())
                .accessibilityIdentifier("speaker-test")
                .accessibilityValue(speakerTest.playing ? "Playing" : "")
            if let error = speakerTest.error { Text(error).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error) }
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
                HStack { Text("Voice enhancement"); Spacer(); Text("\(voice.voiceProcessingStrength)%") }.font(CaperTheme.font(12)).monospacedDigit()
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
                // Web's mic-test-button: the card's one primary (eggshell) action.
                .buttonStyle(CaperPrimaryButton()).frame(width: 160)
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
            if let error = micTest.error { Text(error).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error) }
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
                Text("No audible signal detected. Check your mic and try again.").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
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
                Button(action: close) { CaperIcon(name: "x") }
                    .buttonStyle(SidebarIconButton()).accessibilityLabel("Close connection details").keyboardShortcut(.cancelAction)
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
