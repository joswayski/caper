import AppKit
import CaperCore

@MainActor final class SignInController: NSViewController, NSTextFieldDelegate {
    private let model: AppModel
    private let observation = NativeObservation()
    private let heading = NativeStyle.label("Welcome to Caper", size: 30, bold: true)
    private let email = NSTextField()
    private let code = NSTextField()
    private let username = NSTextField()
    private let name = NSTextField()
    private let error = NativeStyle.label("")
    private lazy var submit = NativeButton("Continue", primary: true) { [weak self] in self?.submitForm() }
    private lazy var resend: NativeButton = NativeButton("Resend code") { [weak self] in
        guard let self, self.resend.isEnabled else { return }
        self.sendNewCode()
    }
    private lazy var changeEmail = NativeButton("Use a different email") { [weak self] in
        self?.model.challengeID = nil; self?.model.error = nil; self?.code.stringValue = ""
    }
    private var timer: Timer?
    private var hadChallenge = false

    init(model: AppModel) { self.model = model; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { timer?.invalidate() }

    override func loadView() { view = NativeView(NativeStyle.blackout) }
    override func viewDidLoad() {
        super.viewDidLoad()
        for (field, placeholder) in [(email, "Email address"), (code, "Sign-in code"),
                                     (username, "Username"), (name, "Display name")] {
            field.placeholderString = placeholder
            field.setAccessibilityLabel(placeholder)
            field.font = NativeStyle.font(16)
            field.delegate = self
            field.target = self; field.action = #selector(enterPressed)
            view.addSubview(field)
        }
        error.textColor = NSColor(CaperTheme.error)
        error.maximumNumberOfLines = 4
        error.lineBreakMode = .byWordWrapping
        error.setAccessibilityIdentifier("sign-in-error")
        [heading, error, submit, resend, changeEmail].forEach { view.addSubview($0) }
        observation.start { [weak self] in self?.update() }
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.updateResend() }
        }
    }
    override func viewDidLayout() {
        super.viewDidLayout()
        let width = min(360, max(0, view.bounds.width - 48))
        let x = (view.bounds.width - width) / 2
        let fields = [email, code, username, name].filter { !$0.isHidden }
        let controls = [resend, changeEmail].filter { !$0.isHidden }
        let height = 64 + CGFloat(fields.count) * 48 + 52 + CGFloat(controls.count) * 42 + 80
        var y = max(40, (view.bounds.height - height) / 2)
        heading.frame = NSRect(x: x, y: y, width: min(400, view.bounds.width - x - 24), height: 44)
        y += 64
        for field in fields {
            field.frame = NSRect(x: x, y: y, width: width, height: 36)
            y += 48
        }
        submit.frame = NSRect(x: x, y: y, width: width, height: 40)
        y += 52
        for control in controls {
            control.frame = NSRect(x: x, y: y, width: width, height: 32)
            y += 42
        }
        error.frame = NSRect(x: x, y: y, width: width, height: 80)
    }
    private func update() {
        let onboarding = model.phase == .onboarding
        let challenge = model.challengeID != nil
        heading.stringValue = onboarding ? "Make yourself at home" : challenge ? "Check your email" : "Welcome to Caper"
        email.isHidden = onboarding || challenge
        code.isHidden = onboarding || !challenge
        username.isHidden = !onboarding; name.isHidden = !onboarding
        let exhausted = challenge && model.loginAttemptsRemaining == 0
        let resendState = NativePresentation.resend(secondsSince: model.codeSentAt, codesSent: model.codesSent, now: Date())
        changeEmail.isHidden = onboarding || !challenge
        resend.isHidden = changeEmail.isHidden || exhausted || !resendState.allowed
        submit.title = model.busy ? "Please wait…" : onboarding ? "Save profile" : exhausted ? "Email me a new code" : challenge ? "Sign in" : "Send code"
        submit.setAccessibilityLabel(submit.title)
        let valid = onboarding ? !username.stringValue.isEmpty && !name.stringValue.isEmpty
            : exhausted ? resendState.allowed
            : challenge ? NativePresentation.signInCode(code.stringValue).count == 6 : !email.stringValue.isEmpty
        submit.isEnabled = !model.busy && valid
        for field in [email, code, username, name] { field.isEnabled = !model.busy }
        code.isEnabled = !model.busy && !exhausted
        changeEmail.isEnabled = !model.busy
        error.stringValue = model.error ?? ""
        view.needsLayout = true
        updateResend()
        if challenge != hadChallenge {
            hadChallenge = challenge
            view.window?.makeFirstResponder(challenge ? code : email)
        }
    }
    private func updateResend() {
        let state = NativePresentation.resend(secondsSince: model.codeSentAt, codesSent: model.codesSent, now: Date())
        resend.title = state.label
        resend.setAccessibilityLabel(state.label)
        resend.isEnabled = state.enabled && !model.busy
    }
    func controlTextDidChange(_ notification: Notification) {
        guard !model.busy else { return }
        if model.loginAttemptsRemaining != 0 { model.error = nil }
        if let field = notification.object as? NSTextField, field === code {
            let normalized = NativePresentation.signInCode(code.stringValue)
            if normalized != code.stringValue {
                DispatchQueue.main.async { [weak self] in
                    guard let self, !self.model.busy else { return }
                    self.code.stringValue = normalized
                    self.update()
                }
            }
        }
        update()
    }
    @objc private func enterPressed() { submitForm() }
    private func submitForm() {
        guard submit.isEnabled else { return }
        if model.phase == .onboarding {
            Task { await model.saveProfile(username: username.stringValue, displayName: name.stringValue) }
        } else if model.challengeID == nil {
            Task { await model.requestCode(email: email.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)) }
        } else if model.loginAttemptsRemaining == 0 {
            sendNewCode()
        } else {
            let normalized = NativePresentation.signInCode(code.stringValue)
            guard normalized.count == 6 else { return }
            Task { await model.verify(code: normalized) }
        }
    }
    private func sendNewCode() {
        let sent = model.codesSent
        Task {
            await model.requestCode(email: email.stringValue.trimmingCharacters(in: .whitespacesAndNewlines))
            if model.codesSent > sent { code.stringValue = ""; update() }
        }
    }
}
