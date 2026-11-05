import CaperCore
import UIKit

/// Email sign-in, the same two steps as web and the SwiftUI app: request a
/// six-character code, then enter it.
final class SignInViewController: UIViewController, UITextFieldDelegate {
    private let model: AppModel
    private let scroll = UIScrollView()
    private let stack = UIStackView()
    private let heading = UIKitStyle.label(size: 52, weight: .black)
    private let lede = UIKitStyle.label(size: 16, color: UIKitStyle.muted)
    private let fieldTitle = UIKitStyle.label(size: 12, weight: .bold)
    private let field = CaperTextField()
    private let errorBox = UIView()
    private let errorLabel = UIKitStyle.label(size: 14)
    private let note = UIKitStyle.label(size: 14)
    private lazy var primary = CaperButton("Email me a code", icon: "arrow-right") { [weak self] in self?.submit() }
    private lazy var differentEmail = CaperButton("Use a different email", kind: .plain, size: 14) { [weak self] in self?.useDifferentEmail() }
    private lazy var resend = CaperButton("Resend code", kind: .plain, size: 14) { [weak self] in self?.sendNewCode() }
    private let observer = ModelObserver()
    private var email = ""
    private var resent = false
    private var ticker: Timer?

    init(model: AppModel) {
        self.model = model
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIKitStyle.blackout
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.keyboardDismissMode = .interactive
        view.addSubview(scroll)
        stack.axis = .vertical
        stack.alignment = .fill
        stack.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(stack)

        let wordmark = UIImageView(image: UIImage(named: "CaperWordmark"))
        wordmark.contentMode = .scaleAspectFit
        wordmark.isAccessibilityElement = true
        wordmark.accessibilityLabel = "Caper"
        wordmark.accessibilityTraits = .image
        wordmark.translatesAutoresizingMaskIntoConstraints = false
        let wordmarkRow = UIView()
        wordmarkRow.addSubview(wordmark)
        NSLayoutConstraint.activate([
            wordmark.widthAnchor.constraint(equalToConstant: 132), wordmark.heightAnchor.constraint(equalToConstant: 35),
            wordmark.leadingAnchor.constraint(equalTo: wordmarkRow.leadingAnchor),
            wordmark.topAnchor.constraint(equalTo: wordmarkRow.topAnchor),
            wordmark.bottomAnchor.constraint(equalTo: wordmarkRow.bottomAnchor),
        ])

        heading.numberOfLines = 0
        heading.accessibilityTraits = .header
        lede.numberOfLines = 0
        note.numberOfLines = 0
        field.delegate = self
        field.addAction(UIAction { [weak self] _ in self?.edited() }, for: .editingChanged)

        errorBox.backgroundColor = UIKitStyle.surface
        errorBox.layer.cornerRadius = 8
        errorBox.layer.borderWidth = 1
        errorBox.layer.borderColor = UIKitStyle.terracottaBright.cgColor
        errorLabel.numberOfLines = 0
        errorBox.addSubview(errorLabel)
        NSLayoutConstraint.activate([
            errorLabel.leadingAnchor.constraint(equalTo: errorBox.leadingAnchor, constant: 14),
            errorLabel.trailingAnchor.constraint(equalTo: errorBox.trailingAnchor, constant: -14),
            errorLabel.topAnchor.constraint(equalTo: errorBox.topAnchor, constant: 14),
            errorLabel.bottomAnchor.constraint(equalTo: errorBox.bottomAnchor, constant: -14),
        ])
        errorBox.isAccessibilityElement = true
        errorBox.accessibilityIdentifier = "login-error"

        let primaryRow = UIView()
        primaryRow.addSubview(primary)
        NSLayoutConstraint.activate([
            primary.trailingAnchor.constraint(equalTo: primaryRow.trailingAnchor),
            primary.topAnchor.constraint(equalTo: primaryRow.topAnchor),
            primary.bottomAnchor.constraint(equalTo: primaryRow.bottomAnchor),
            primary.heightAnchor.constraint(equalToConstant: 58),
            primary.widthAnchor.constraint(greaterThanOrEqualToConstant: 200),
            primary.leadingAnchor.constraint(greaterThanOrEqualTo: primaryRow.leadingAnchor),
        ])
        primary.accessibilityIdentifier = "login-primary"
        resend.accessibilityIdentifier = "login-resend-code"
        let links = UIStackView(arrangedSubviews: [differentEmail, resend, UIView()])
        links.spacing = 24
        links.alignment = .firstBaseline

        for (view, gap) in [(wordmarkRow, 58.0), (heading, 4), (lede, 20), (fieldTitle, 7), (field, 18), (errorBox, 12),
                            (note, 12), (primaryRow, 18), (links, 0)] as [(UIView, CGFloat)] {
            stack.addArrangedSubview(view)
            stack.setCustomSpacing(gap, after: view)
        }
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
            stack.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 32),
            stack.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -32),
            stack.centerXAnchor.constraint(equalTo: scroll.frameLayoutGuide.centerXAnchor),
            stack.widthAnchor.constraint(lessThanOrEqualToConstant: 440),
            stack.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor, constant: -40).withPriority(.required - 1),
            field.heightAnchor.constraint(equalToConstant: 42),
        ])
        observer.start { [weak self] in self?.render() }
        ticker = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.renderResend() }
        }
    }

    deinit { ticker?.invalidate() }

    /// Wraps between words like SwiftUI's Text instead of truncating.
    private static let headingParagraph: NSParagraphStyle = {
        let style = NSMutableParagraphStyle()
        style.lineBreakMode = .byWordWrapping
        return style
    }()

    private var codeStep: Bool { model.challengeID != nil }
    /// Past the resend limit no code would arrive.
    private var resendExhausted: Bool { !NativePresentation.resend(secondsSince: nil, codesSent: model.codesSent, now: Date()).allowed }

    private func render() {
        let busy = model.busy
        let code = codeStep
        // The display heading's tight tracking, as on web and SwiftUI.
        heading.attributedText = NSAttributedString(string: code ? "Check your email." : "Welcome to Caper",
                                                    attributes: [.kern: -2.5, .font: heading.font as Any, .paragraphStyle: Self.headingParagraph])
        let address = email.trimmingCharacters(in: .whitespacesAndNewlines)
        lede.text = code ? "Enter the six-character code sent to \(address). It expires in 10 minutes."
            : "Use your email to create an account or log back in."
        fieldTitle.text = code ? "Sign-in code" : "Email address"
        if field.step != code {
            field.step = code
            field.text = ""
            field.placeholder = code ? "e.g. ABC234" : "you@example.com"
            field.keyboardType = code ? .asciiCapable : .emailAddress
            field.textContentType = code ? .oneTimeCode : .emailAddress
            field.autocapitalizationType = code ? .allCharacters : .none
            field.autocorrectionType = .no
            field.returnKeyType = code ? .go : .send
            field.accessibilityLabel = code ? "Sign-in code" : "Email address"
            field.accessibilityIdentifier = code ? "login-code" : "login-email"
            field.reloadInputViews()
            if field.isFirstResponder || view.window != nil { field.becomeFirstResponder() }
        }
        let exhausted = code && model.loginAttemptsRemaining == 0
        field.isEnabled = !exhausted
        if let error = model.error {
            errorLabel.text = error
            errorBox.accessibilityLabel = error
            errorBox.isHidden = false
            UIAccessibility.post(notification: .announcement, argument: error)
        } else { errorBox.isHidden = true }

        var notes: [String] = []
        if code && model.loginAttemptsRemaining == 1 { notes.append("One attempt left. Check the code carefully.") }
        if resent && model.error == nil { notes.append("We sent a new code. Earlier codes no longer work.") }
        if code && resendExhausted { notes.append("Still nothing? Check your spam folder, or try again in 15 minutes.") }
        note.text = notes.joined(separator: "\n\n")
        note.isHidden = notes.isEmpty

        let title: String, busyTitle: String
        if !code { (title, busyTitle) = ("Email me a code", "Sending…") }
        else if exhausted { (title, busyTitle) = ("Email me a new code", "Sending…") }
        else { (title, busyTitle) = ("Continue", "Checking…") }
        primary.setTitle(busy ? busyTitle : title)
        primary.configuration?.showsActivityIndicator = busy
        primary.isHidden = exhausted && resendExhausted
        differentEmail.isHidden = !code
        differentEmail.isEnabled = !busy
        renderPrimaryEnabled()
        renderResend()
    }

    private func renderPrimaryEnabled() {
        let text = field.text ?? ""
        if model.busy { primary.isEnabled = false }
        else if !codeStep { primary.isEnabled = !text.isEmpty }
        else if model.loginAttemptsRemaining == 0 { primary.isEnabled = true }
        else { primary.isEnabled = text.count == 6 }
    }

    private func renderResend() {
        let state = NativePresentation.resend(secondsSince: model.codeSentAt, codesSent: model.codesSent, now: Date())
        resend.isHidden = !codeStep || !state.allowed || model.loginAttemptsRemaining == 0
        resend.setTitle(state.label)
        resend.isEnabled = state.enabled && !model.busy
    }

    private func edited() {
        if codeStep {
            // The unambiguous code alphabet, uppercased, six characters.
            let filtered = NativePresentation.signInCode(field.text ?? "")
            if field.text != filtered { field.text = filtered }
        } else {
            email = field.text ?? ""
        }
        if model.error != nil, !(codeStep && model.loginAttemptsRemaining == 0) { model.error = nil }
        renderPrimaryEnabled()
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        submit()
        return false
    }

    /// A request in flight keeps focus but takes no edits, like web's read-only input.
    func textField(_ textField: UITextField, shouldChangeCharactersIn range: NSRange, replacementString string: String) -> Bool {
        !model.busy
    }

    private func submit() {
        guard !model.busy else { return }
        if !codeStep {
            guard !email.isEmpty else { return }
            Task { await model.requestCode(email: email) }
        } else if model.loginAttemptsRemaining == 0 {
            sendNewCode()
        } else {
            let code = field.text ?? ""
            guard code.count == 6 else { return }
            Task { await model.verify(code: code) }
        }
    }

    private func useDifferentEmail() {
        resent = false
        model.challengeID = nil
        model.error = nil
    }

    private func sendNewCode() {
        guard !model.busy, codeStep, !resendExhausted else { return }
        let sent = model.codesSent
        resent = false
        Task {
            await model.requestCode(email: email)
            guard model.codesSent > sent, model.challengeID != nil, model.error == nil else { return }
            field.text = ""
            resent = true
            render()
        }
    }
}

final class CaperTextField: UITextField {
    var step = false
    override init(frame: CGRect) {
        super.init(frame: frame)
        font = UIKitStyle.font(14)
        adjustsFontForContentSizeCategory = true
        textColor = UIKitStyle.text
        tintColor = UIKitStyle.terracottaBright
        backgroundColor = UIKitStyle.composer
        layer.cornerRadius = 8
        layer.borderWidth = 1
        layer.borderColor = UIKitStyle.border.cgColor
        keyboardAppearance = .dark
        placeholder = "you@example.com"
        keyboardType = .emailAddress
        textContentType = .emailAddress
        autocapitalizationType = .none
        autocorrectionType = .no
        returnKeyType = .send
        accessibilityLabel = "Email address"
        accessibilityIdentifier = "login-email"
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var placeholder: String? {
        didSet {
            attributedPlaceholder = placeholder.map {
                NSAttributedString(string: $0, attributes: [.foregroundColor: UIKitStyle.muted.withAlphaComponent(0.6), .font: UIKitStyle.font(14)])
            }
        }
    }
    override func textRect(forBounds bounds: CGRect) -> CGRect { bounds.insetBy(dx: 11, dy: 0) }
    override func editingRect(forBounds bounds: CGRect) -> CGRect { bounds.insetBy(dx: 11, dy: 0) }
}
