import CoreText
import Foundation

public enum CaperFontLoader {
    private static let names = [
        "Figtree-Regular",
        "Figtree-Medium",
        "Figtree-Bold",
        "Figtree-Black",
        "BricolageGrotesque-Bold",
        "BricolageGrotesque-ExtraBold",
    ]
    private static var registered = false

    public static func register() {
        guard !registered else { return }
        registered = true
        for name in names {
            guard let url = Bundle.main.url(forResource: name, withExtension: "ttf", subdirectory: "Fonts")
                ?? Bundle.main.url(forResource: name, withExtension: "ttf") else { continue }
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        }
    }
}
