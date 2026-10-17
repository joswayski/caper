import Foundation

/// Design H's "Live now" rules: your call's card, the other live rooms and the
/// phone strip share them, so the sidebar and its tests agree.
enum LiveNow {
    /// Room cards shown before the rest fold into "N more live".
    static let shownRooms = 2
    /// Faces your call's card shows before the rest fold into a "+N" tile.
    static let maximumFaces = 10
    /// Rows of faces your call's card may use.
    static let faceRows = 2

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

    /// You first, then everyone else in the order they joined. Speaking never
    /// reorders faces.
    static func selfFirst<Person>(_ people: [Person], isSelf: (Person) -> Bool) -> [Person] {
        people.filter(isSelf) + people.filter { !isSelf($0) }
    }

    /// Other live rooms: busiest first, then by name; ties keep their order.
    static func busiestFirst<Room>(_ rooms: [Room], count: (Room) -> Int, name: (Room) -> String) -> [Room] {
        rooms.enumerated().sorted { a, b in
            let left = count(a.element), right = count(b.element)
            if left != right { return left > right }
            let order = name(a.element).localizedStandardCompare(name(b.element))
            if order != .orderedSame { return order == .orderedAscending }
            return a.offset < b.offset
        }.map(\.element)
    }
}
