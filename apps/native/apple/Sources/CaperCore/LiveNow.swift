import Foundation

/// Design H's voice rules ("Live now"): your call's card, the other live rooms'
/// cards and the phone strip share them, so the sidebar and its tests agree.
enum LiveNow {
    /// Faces your call's card shows before the rest fold into a "+N" tile.
    static let maximumFaces = 10
    /// Rows of faces your call's card may use.
    static let faceRows = 2
    /// Faces the phone strip shows before the rest fold into a "+N" chip.
    static let stripFaces = 5

    /// A room's one-line names: "Maya"; "Maya and Alex"; "Maya, Alex and Sam";
    /// "Maya, Alex and 3 more".
    static func names(_ names: [String]) -> String {
        switch names.count {
        case 0: return ""
        case 1: return names[0]
        case 2: return "\(names[0]) and \(names[1])"
        case 3: return "\(names[0]), \(names[1]) and \(names[2])"
        default: return "\(names[0]), \(names[1]) and \(names.count - 2) more"
        }
    }

    /// "1 person", "12 people".
    static func people(_ count: Int) -> String { count == 1 ? "1 person" : "\(count) people" }

    /// How many faces, `pitch` apart, fit across `width` (at least one).
    static func columns(width: Double, pitch: Double) -> Int {
        guard width.isFinite, pitch > 0 else { return 1 }
        return max(1, Int((width / pitch).rounded(.down)))
    }

    /// Face slots in at most two rows of `columns`, and never more than ten.
    static func faceSlots(columns: Int) -> Int { min(maximumFaces, max(1, columns) * faceRows) }

    /// The faces shown, and those folded into "+N": everyone when they fit,
    /// else one face fewer than the slots, which leaves the tile its own slot.
    static func fold<Person>(_ people: [Person], slots: Int) -> (shown: [Person], folded: [Person]) {
        guard people.count > slots else { return (people, []) }
        let kept = max(0, slots - 1)
        return (Array(people.prefix(kept)), Array(people.dropFirst(kept)))
    }

    /// The phone strip's faces: the card's people in the card's order, up to
    /// five, else the first four and a "+N" chip for the rest (total − 4).
    static func stripFold<Person>(_ people: [Person]) -> (shown: [Person], folded: [Person]) {
        fold(people, slots: stripFaces)
    }

    /// You first, then everyone else in the order they joined. Speaking never
    /// reorders faces.
    static func selfFirst<Person>(_ people: [Person], isSelf: (Person) -> Bool) -> [Person] {
        people.filter(isSelf) + people.filter { !isSelf($0) }
    }

    /// The channel rows listed: all of them, or while Channels is collapsed only
    /// the live ones (your call's and rooms with people in voice), in list order,
    /// so collapsing never hides voice.
    static func listed<Row>(_ rows: [Row], expanded: Bool, live: (Row) -> Bool) -> [Row] {
        expanded ? rows : rows.filter(live)
    }

    /// Whether a call's channel has a row in the listed space, where its card
    /// grows in place; otherwise its card sits above the channels.
    static func hasRow(_ channelID: String, rows: [String]) -> Bool { rows.contains(channelID) }
}
