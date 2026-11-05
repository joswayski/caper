import CaperCore
import UIKit

/// A new account picks its username and display name before entering Caper.
final class OnboardingViewController: UIViewController, UITextFieldDelegate {
    private let model: AppModel
    private let username = CaperTextField()
    private let displayName = CaperTextField()
    private let error = UIKitStyle.label(size: 14, color: UIKitStyle.error)
    private lazy var save = CaperButton("Continue", icon: "arrow-right") { [weak self] in self?.submit() }
    private let observer = ModelObserver()

    init(model: AppModel) {
        self.model = model
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIKitStyle.blackout
        let heading = UIKitStyle.label("Make your profile", size: 34, weight: .black)
        heading.numberOfLines = 0
        heading.accessibilityTraits = .header
        let lede = UIKitStyle.label("Pick a username people can find you by, and the name they see.", size: 16, color: UIKitStyle.muted)
        lede.numberOfLines = 0
        configure(username, title: "Username", placeholder: "e.g. maya_r", identifier: "onboarding-username")
        username.textContentType = .username
        username.returnKeyType = .next
        configure(displayName, title: "Display name", placeholder: "e.g. Maya", identifier: "onboarding-display-name")
        displayName.textContentType = .name
        displayName.autocapitalizationType = .words
        displayName.returnKeyType = .go
        displayName.text = model.account?.displayName
        error.numberOfLines = 0
        save.heightAnchor.constraint(equalToConstant: 58).isActive = true
        let stack = UIStackView(arrangedSubviews: [heading, lede, label("Username"), username, label("Display name"), displayName, error, save])
        stack.axis = .vertical
        stack.spacing = 12
        stack.setCustomSpacing(7, after: stack.arrangedSubviews[2])
        stack.setCustomSpacing(7, after: stack.arrangedSubviews[4])
        stack.translatesAutoresizingMaskIntoConstraints = false
        let scroll = UIScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.keyboardDismissMode = .interactive
        scroll.addSubview(stack)
        view.addSubview(scroll)
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
            username.heightAnchor.constraint(equalToConstant: 42),
            displayName.heightAnchor.constraint(equalToConstant: 42),
        ])
        observer.start { [weak self] in
            guard let self else { return }
            self.error.text = self.model.error
            self.error.isHidden = self.model.error == nil
            self.save.isEnabled = !self.model.busy
            self.save.setTitle(self.model.busy ? "Saving…" : "Continue")
        }
    }

    private func label(_ text: String) -> UILabel { UIKitStyle.label(text, size: 12, weight: .bold) }

    private func configure(_ field: CaperTextField, title: String, placeholder: String, identifier: String) {
        field.placeholder = placeholder
        field.keyboardType = .default
        field.accessibilityLabel = title
        field.accessibilityIdentifier = identifier
        field.delegate = self
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        if textField === username { displayName.becomeFirstResponder() } else { submit() }
        return false
    }

    private func submit() {
        let name = username.text ?? "", display = displayName.text ?? ""
        Task { await model.saveProfile(username: name, displayName: display) }
    }
}
