import Foundation
#if os(macOS)
import AppKit
import Quartz

@MainActor final class AppKitFilePreview: NSViewController {
    private let url: URL
    private let close: () -> Void
    init(url: URL, close: @escaping () -> Void) { self.url = url; self.close = close; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() {
        view = NSView(); view.wantsLayer = true; view.layer?.backgroundColor = AppKitKit.surface.cgColor
        let done = AppKitKitButton("Close preview", action: close); done.keyEquivalent = "\u{1b}"
        let preview = QLPreviewView(frame: .zero, style: .normal)!
        preview.previewItem = url as NSURL; preview.autostarts = true
        for child in [done, preview] { child.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(child) }
        NSLayoutConstraint.activate([
            done.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -12), done.topAnchor.constraint(equalTo: view.topAnchor, constant: 12),
            preview.leadingAnchor.constraint(equalTo: view.leadingAnchor), preview.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            preview.topAnchor.constraint(equalTo: done.bottomAnchor, constant: 12), preview.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
    }
}
#elseif os(iOS)
import UIKit
import QuickLook

@MainActor final class UIKitFilePreview: UINavigationController, QLPreviewControllerDataSource {
    private let url: URL
    private let close: () -> Void
    init(url: URL, close: @escaping () -> Void) {
        self.url = url; self.close = close
        let preview = QLPreviewController()
        super.init(rootViewController: preview); preview.dataSource = self
        preview.navigationItem.leftBarButtonItem = UIBarButtonItem(title: "Close preview", primaryAction: UIAction { _ in close() })
        modalPresentationStyle = .fullScreen
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func numberOfPreviewItems(in controller: QLPreviewController) -> Int { 1 }
    func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem { url as NSURL }
}
#endif
