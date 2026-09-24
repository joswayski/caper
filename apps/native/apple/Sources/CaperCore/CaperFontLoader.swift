import CoreText
import Foundation

public enum CaperFontLoader {
    private static let names = ["Satoshi-Regular", "Satoshi-Medium", "Satoshi-Bold", "Satoshi-Black"]
    private static var registered = false

    public static func register() {
        guard !registered else { return }
        registered = true
        for name in names {
            guard let url = Bundle.main.url(forResource: name, withExtension: "otf", subdirectory: "Fonts")
                ?? Bundle.main.url(forResource: name, withExtension: "otf") else { continue }
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        }
    }
}
