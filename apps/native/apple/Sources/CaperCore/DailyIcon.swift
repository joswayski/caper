import Foundation

/// Shared selection for in-app wordmarks and the running macOS Dock icon.
/// Account avatar IDs are deliberately unrelated to this installation-local choice.
public enum CaperDailyIcon {
    public static let count = 800

    public static func current(now: Date = Date(), defaults: UserDefaults = .standard,
                               random: UInt64 = UInt64.random(in: UInt64.min...UInt64.max)) -> Int {
        // Retain the existing macOS keys; iOS uses them only for in-app branding.
        let day = utcDay(containing: now)
        let savedDay = defaults.string(forKey: "daily-dock-icon-day-v1")
        let savedIndex = defaults.object(forKey: "daily-dock-icon-index-v1") as? Int
        let index = select(day: day, savedDay: savedDay, savedIndex: savedIndex, random: random)
        if day != savedDay || index != savedIndex {
            defaults.set(day, forKey: "daily-dock-icon-day-v1")
            defaults.set(index, forKey: "daily-dock-icon-index-v1")
        }
        return index
    }

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
