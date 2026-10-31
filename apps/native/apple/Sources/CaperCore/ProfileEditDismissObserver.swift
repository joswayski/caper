#if os(iOS)
import SwiftUI
import UIKit

/// SwiftUI can prevent a sheet swipe, but does not report the attempted swipe.
/// Keep its presentation delegate for normal dismissal and observe the blocked
/// attempt so the profile editor can offer Keep editing / Discard.
struct ProfileEditDismissObserver: UIViewControllerRepresentable {
    let preventDismissal: Bool
    let attempt: () -> Void

    func makeUIViewController(context: Context) -> Controller { Controller() }
    func updateUIViewController(_ controller: Controller, context: Context) {
        controller.preventDismissal = preventDismissal
        controller.attempt = attempt
        DispatchQueue.main.async { controller.install() }
    }
    static func dismantleUIViewController(_ controller: Controller, coordinator: ()) { controller.uninstall() }

    final class Controller: UIViewController, UIAdaptivePresentationControllerDelegate {
        var preventDismissal = false
        var attempt: (() -> Void)?
        private weak var presentation: UIPresentationController?
        private weak var previous: UIAdaptivePresentationControllerDelegate?

        override func loadView() {
            view = UIView()
            view.isUserInteractionEnabled = false
        }
        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            install()
        }
        func install() {
            var ancestor = parent
            while let controller = ancestor {
                if let presentation = controller.presentationController, presentation.containerView != nil {
                    if presentation.delegate !== self {
                        previous = presentation.delegate
                        self.presentation = presentation
                        presentation.delegate = self
                    }
                    return
                }
                ancestor = controller.parent
            }
        }
        func uninstall() {
            if presentation?.delegate === self { presentation?.delegate = previous }
        }
        func presentationControllerShouldDismiss(_ presentationController: UIPresentationController) -> Bool {
            !preventDismissal && (previous?.presentationControllerShouldDismiss?(presentationController) ?? true)
        }
        func presentationControllerDidAttemptToDismiss(_ presentationController: UIPresentationController) { attempt?() }
        func presentationControllerWillDismiss(_ presentationController: UIPresentationController) {
            previous?.presentationControllerWillDismiss?(presentationController)
        }
        func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
            previous?.presentationControllerDidDismiss?(presentationController)
        }
    }
}
#endif
