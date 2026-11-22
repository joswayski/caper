import CoreText
import Foundation

public enum CaperFontLoader {
    private static let names = ["Inrond40-Regular", "Inrond40-Medium", "Inrond40-Bold", "Inrond40-Black"]
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
