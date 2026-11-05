import Foundation

/// Estimated geometry for native virtualized timelines. Text is measured by the
/// host only when a row is realized; prefix sums locate the viewport without
/// walking or creating accessibility elements for the rest of the history.
public struct NativeTimelineLayout {
    public struct Anchor: Equatable {
        public let id: String
        public let offset: CGFloat
    }

    public private(set) var ids: [String] = []
    public private(set) var width: CGFloat = 0
    private var indices: [String: Int] = [:]
    private var heights: [CGFloat] = []
    private var sums: [CGFloat] = [0]
    private var measured: [String: CGFloat] = [:]
    private let estimate: CGFloat

    public init(estimatedHeight: CGFloat = 76) {
        precondition(estimatedHeight.isFinite && estimatedHeight > 0)
        estimate = estimatedHeight
    }

    public var contentHeight: CGFloat { origin(of: ids.count) }
    public var measuredCount: Int { measured.count }

    /// Keep heights for unchanged IDs at the same width. The caller invalidates
    /// changed content, including a row whose grouping changes after a prepend.
    public mutating func replace(ids: [String], width: CGFloat, invalidating: Set<String> = []) {
        precondition(Set(ids).count == ids.count)
        let width = max(1, width)
        if self.width != width { measured.removeAll(keepingCapacity: true) }
        self.width = width
        self.ids = ids
        indices = Dictionary(uniqueKeysWithValues: ids.enumerated().map { ($0.element, $0.offset) })
        measured = measured.filter { indices[$0.key] != nil && !invalidating.contains($0.key) }
        heights = ids.map { measured[$0] ?? estimate }
        sums = [0] + heights
        // Build a Fenwick tree in linear time, without measuring any text.
        for index in 1..<sums.count {
            let parent = index + (index & -index)
            if parent < sums.count { sums[parent] += sums[index] }
        }
    }

    public func isMeasured(_ index: Int) -> Bool { measured[ids[index]] != nil }
    public func height(at index: Int) -> CGFloat { heights[index] }

    public func origin(of index: Int) -> CGFloat {
        var cursor = min(max(0, index), ids.count)
        var total: CGFloat = 0
        while cursor > 0 {
            total += sums[cursor]
            cursor -= cursor & -cursor
        }
        return total
    }

    public mutating func measure(_ index: Int, height: CGFloat) {
        guard heights.indices.contains(index), height.isFinite, height > 0 else { return }
        let delta = height - heights[index]
        heights[index] = height
        measured[ids[index]] = height
        var cursor = index + 1
        while cursor < sums.count {
            sums[cursor] += delta
            cursor += cursor & -cursor
        }
    }

    /// First row whose bottom is strictly below the offset (half-open rows).
    public func index(at offset: CGFloat) -> Int {
        guard !ids.isEmpty else { return 0 }
        let target = max(0, offset)
        var index = 0
        var total: CGFloat = 0
        var bit = 1
        while bit <= ids.count / 2 { bit <<= 1 }
        while bit > 0 {
            let next = index + bit
            if next <= ids.count, total + sums[next] <= target {
                index = next
                total += sums[next]
            }
            bit >>= 1
        }
        return min(index, ids.count - 1)
    }

    public func visibleRange(offset: CGFloat, height: CGFloat, overscan: CGFloat = 0) -> Range<Int> {
        guard !ids.isEmpty, height > 0 else { return 0..<0 }
        let start = max(0, offset - overscan)
        let end = min(contentHeight, offset + height + overscan)
        guard start < end else { return 0..<0 }
        let first = index(at: start)
        let last = index(at: end)
        return first..<(origin(of: last) == end ? last : last + 1)
    }

    public func anchor(at offset: CGFloat) -> Anchor? {
        guard !ids.isEmpty else { return nil }
        let index = index(at: offset)
        return Anchor(id: ids[index], offset: offset - origin(of: index))
    }

    public func offset(for anchor: Anchor) -> CGFloat? {
        indices[anchor.id].map { origin(of: $0) + min(anchor.offset, heights[$0]) }
    }
}
