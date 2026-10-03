import Foundation

/// Pure selection policy shared by the macOS icon controller and its tests.
/// Account avatar IDs are deliberately unrelated to this installation-local choice.
public enum CaperDailyIcon {
    public static let count = 800

    public static func utcDay(containing date: Date) -> String {
        let formatter = DateFormatter()
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter.string(from: date)
    }

    public static func select(day: String, savedDay: String?, savedIndex: Int?, random: UInt64) -> Int {
        if day == savedDay, let savedIndex, (0..<count).contains(savedIndex) { return savedIndex }
        if let savedIndex, (0..<count).contains(savedIndex) {
            let candidate = Int(random % UInt64(count - 1))
            return candidate >= savedIndex ? candidate + 1 : candidate
        }
        return Int(random % UInt64(count))
    }
}
