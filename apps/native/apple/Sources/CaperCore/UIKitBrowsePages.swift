#if os(iOS)
import UIKit
import Observation

/// Browse's two pushed inboxes keep the original model's navigation and paging
/// contracts. Reading a message request never accepts it.
@MainActor public final class UIKitBrowsePage: UIViewController {
    public enum Kind { case requests, feedback }
    private let model: AppModel
    private let kind: Kind
    private let scroll = UIScrollView()
    private let body = UIStackView()
    private let refresh = UIRefreshControl()
    private var generation = 0
    private var clock: Timer?
    private var rows: [String: BrowseInboxButton] = [:]
    private let emptyState = FeedbackEmptyState()

    public init(model: AppModel, kind: Kind) {
        self.model = model; self.kind = kind
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    public override func loadView() {
        view = scroll; view.backgroundColor = UIColor(CaperTheme.sidebar)
        view.translatesAutoresizingMaskIntoConstraints = false
        view.accessibilityIdentifier = kind == .requests ? "message-requests-page" : "feedback-page"
        body.axis = .vertical; body.spacing = 4; body.translatesAutoresizingMaskIntoConstraints = false
        body.accessibilityIdentifier = kind == .requests ? "message-requests-list" : "feedback-list"
        scroll.addSubview(body)
        NSLayoutConstraint.activate([
            body.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor, constant: 16),
            body.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor, constant: -16),
            body.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 8),
            body.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -8),
            body.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor, constant: -32),
        ])
        if kind == .feedback {
            scroll.refreshControl = refresh
            refresh.addTarget(self, action: #selector(refreshFeedback), for: .valueChanged)
        }
    }
    public override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated); generation += 1; observe(generation)
        if kind == .feedback {
            refreshFeedback()
            clock = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.render() }
            }
        }
    }
    public override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated); generation += 1; clock?.invalidate(); clock = nil
    }
    deinit { clock?.invalidate() }
    private func observe(_ token: Int) {
        guard token == generation else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } }
    }
    @objc private func refreshFeedback() {
        Task { [weak self] in
            guard let self else { return }
            await model.refreshFeedback(); refresh.endRefreshing()
        }
    }
    private func render() {
        let isFeedback = kind == .feedback, archived = model.showingArchivedFeedback
        let back = UIKitMessageDialog.button("Direct messages") { [weak self] in
            guard let self else { return }
            if isFeedback { model.showingFeedback = false } else { model.showingMessageRequests = false }
        }
        // An icon with room to breathe, rather than an arrow character pressed against the words.
        back.configuration?.image = UIKitVoiceControls.icon("arrow-left", size: 16)
        back.configuration?.imagePadding = 10
        back.configuration?.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 14, bottom: 0, trailing: 14)
        back.contentHorizontalAlignment = .leading
        back.accessibilityIdentifier = isFeedback ? "feedback-back" : "message-requests-back"
        var content: [UIView] = [back, UIKitMessageDialog.label(isFeedback
            ? (archived ? FeedbackInbox.archivedTitle : FeedbackInbox.title) : "Message requests", size: 15, bold: true)]
        if !isFeedback || !archived {
            content.append(UIKitMessageDialog.label(isFeedback ? FeedbackInbox.subtitle : "From people you don't share a space with. Accept to reply.", size: 11))
        }
        var next: [String: BrowseInboxButton] = [:]
        if isFeedback {
            if let error = model.feedbackError {
                content.append(UIKitMessageDialog.label(error, error: true))
                let retry = UIKitMessageDialog.button("Retry") { [weak self] in self?.refreshFeedback() }
                retry.accessibilityIdentifier = "feedback-retry"; retry.isEnabled = !model.feedbackLoading; content.append(retry)
            }
            if model.feedbackReports.isEmpty, model.feedbackError == nil {
                // The empty state, like an empty channel's: the day's Caper over a short note. Before the first
                // page arrives the space stays blank rather than flashing "Loading…".
                if model.feedbackLoaded {
                    emptyState.show(archived: archived)
                    content.append(emptyState)
                }
            }
            for report in model.feedbackReports {
                let row = rows[report.id] ?? BrowseInboxButton()
                row.open = { [model] in Task { await model.openFeedback(report) } }
                let actionLabel = archived ? FeedbackInbox.restore : FeedbackInbox.archiveLabel(report)
                row.menu = UIMenu(children: [UIAction(title: actionLabel) { [model] _ in
                    Task { await model.setFeedbackArchived(report, archived: !archived) }
                }])
                row.accessibilityCustomActions = [UIAccessibilityCustomAction(name: actionLabel) { [model] _ in
                    Task { await model.setFeedbackArchived(report, archived: !archived) }; return true
                }]
                row.update(name: report.reporter.displayName, avatarID: report.reporter.avatarId, picture: report.reporter.avatarPicture,
                    subtitle: FeedbackInbox.location(report), preview: FeedbackInbox.preview(report),
                    trailing: FeedbackInbox.relativeTime(report.createdAt, now: .now),
                    status: archived ? nil : FeedbackInbox.status(report), emphasized: !report.answered,
                    selected: model.visit?.channelID == report.channel.id && model.visit?.reportID == report.messageId)
                row.accessibilityLabel = FeedbackInbox.accessibilityLabel(report, now: .now, archived: archived)
                row.accessibilityIdentifier = "feedback-report-\(report.id)"
                next[report.id] = row; content.append(row)
            }
            if model.feedbackCursor != nil {
                let older = UIKitMessageDialog.button(model.feedbackLoading ? "Loading…" : "Load older feedback") { [model] in Task { await model.loadMoreFeedback() } }
                older.accessibilityIdentifier = "feedback-load-older"; older.isEnabled = !model.feedbackLoading; content.append(older)
            }
            let toggle = UIKitMessageDialog.button(archived ? FeedbackInbox.backToFeedback : FeedbackInbox.showArchived) { [model] in
                Task { await model.showArchivedFeedback(!model.showingArchivedFeedback) }
            }
            toggle.accessibilityIdentifier = "feedback-show-archived"; content.append(toggle)
        } else {
            if model.messageRequests.isEmpty { content.append(UIKitMessageDialog.label("No message requests.")) }
            for request in model.messageRequests {
                let row = rows[request.id] ?? BrowseInboxButton()
                row.open = { [model] in Task { await model.select(directMessage: request) } }
                row.update(name: request.peer.displayName, avatarID: request.peer.avatarId, picture: request.peer.avatarPicture,
                    subtitle: "@\(request.peer.username)", selected: model.selectedDirectMessageID == request.id)
                row.accessibilityLabel = "\(request.peer.displayName), @\(request.peer.username)"
                row.accessibilityIdentifier = "message-request-\(request.id)"
                next[request.id] = row; content.append(row)
            }
        }
        rows = next
        for view in body.arrangedSubviews where !content.contains(where: { $0 === view }) { view.removeFromSuperview() }
        for (index, view) in content.enumerated() where !body.arrangedSubviews.indices.contains(index) || body.arrangedSubviews[index] !== view {
            body.removeArrangedSubview(view); body.insertArrangedSubview(view, at: index)
        }
    }
}

/// The feedback inbox with nothing in it: the day's Caper, a heading and a
/// muted line, centered, as an empty channel or DM shows.
@MainActor private final class FeedbackEmptyState: UIStackView {
    private let caper = UIImageView()
    private let heading = UIKitMessageDialog.label("", size: 14, bold: true)
    private let note = UIKitMessageDialog.label("", size: 12)
    init() {
        super.init(frame: .zero)
        axis = .vertical; alignment = .center; spacing = 6
        isLayoutMarginsRelativeArrangement = true
        directionalLayoutMargins = NSDirectionalEdgeInsets(top: 40, leading: 12, bottom: 24, trailing: 12)
        caper.contentMode = .scaleAspectFit; caper.isAccessibilityElement = false
        caper.image = CaperDailyIcon.brandingImage(for: CaperDailyIcon.current())
        caper.widthAnchor.constraint(equalToConstant: 88).isActive = true
        caper.heightAnchor.constraint(equalToConstant: 88).isActive = true
        heading.textAlignment = .center; note.textAlignment = .center; note.numberOfLines = 0
        note.textColor = UIColor(CaperTheme.muted)
        [caper, heading, note].forEach(addArrangedSubview)
        setCustomSpacing(12, after: caper)
        accessibilityIdentifier = "feedback-empty"
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func show(archived: Bool) {
        heading.text = archived ? FeedbackInbox.archivedEmpty : FeedbackInbox.empty
        note.text = archived ? FeedbackInbox.archivedEmptyNote : FeedbackInbox.emptyNote
    }
}

@MainActor private final class BrowseInboxButton: UIButton {
    var open: (() -> Void)?
    private let avatar = UIKitAvatar(side: 32)
    private let name = UIKitMessageDialog.label("", size: 13, bold: true)
    private let subtitle = UIKitMessageDialog.label("", size: 11)
    private let preview = UIKitMessageDialog.label("", size: 12)
    private let trailing = UIKitMessageDialog.label("", size: 10)
    private let status = UIKitMessageDialog.label("", size: 10, bold: true)
    init() {
        super.init(frame: .zero)
        let heading = UIStackView(arrangedSubviews: [name, trailing]); heading.spacing = 6
        let text = UIStackView(arrangedSubviews: [heading, subtitle, preview, status]); text.axis = .vertical; text.spacing = 2
        let row = UIStackView(arrangedSubviews: [avatar, text]); row.spacing = 9; row.alignment = .top
        row.translatesAutoresizingMaskIntoConstraints = false; row.isUserInteractionEnabled = false; addSubview(row)
        name.numberOfLines = 1; subtitle.numberOfLines = 1; preview.numberOfLines = 2
        trailing.setContentCompressionResistancePriority(.required, for: .horizontal)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 9), row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -9),
            row.topAnchor.constraint(equalTo: topAnchor, constant: 6), row.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
            heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
        layer.cornerRadius = 6; isAccessibilityElement = true
        addTarget(self, action: #selector(pressed), for: .touchUpInside)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func pressed() { if !SwipeTapGuard.swiping { open?() } }
    func update(name: String, avatarID: Int?, picture: CaperPicture?, subtitle: String, preview: String = "", trailing: String = "",
                status: String? = nil, emphasized: Bool = false, selected: Bool) {
        avatar.show(name: name, avatarID: avatarID, picture: picture); self.name.text = name; self.subtitle.text = subtitle
        self.preview.text = preview; self.preview.isHidden = preview.isEmpty; self.trailing.text = trailing; self.trailing.isHidden = trailing.isEmpty
        self.status.text = status; self.status.isHidden = status == nil
        self.status.textColor = UIColor(emphasized ? CaperTheme.terracottaBright : CaperTheme.muted)
        backgroundColor = selected ? UIColor(CaperTheme.terracotta).withAlphaComponent(0.16) : .clear
        accessibilityValue = selected ? "Selected" : ""; accessibilityTraits = selected ? [.button, .selected] : .button
    }
}
#endif
