import CoreGraphics
import Foundation

/// Edits to a staged file before it is sent: crop, quarter turns, drawing and
/// text (docs/media.md, "Editing attachments before sending"). The draft keeps
/// its original and this description, so reopening the editor can change any
/// part of it; preparing the upload applies it to the original. The geometry
/// matches web's `apps/web/src/chat/edit.ts` operation for operation and is
/// checked against shared/messages/edit-cases.json.
public struct AttachmentEdit: Equatable, Sendable, Codable {
    /// Clockwise quarter turns of the upright original: 0, 90, 180 or 270.
    public var rotation: Int
    /// Fractions of the rotated, uncropped image's width and height.
    public var crop: EditRect
    /// Painted in order, over the image.
    public var marks: [EditMark]

    public init(rotation: Int = 0, crop: EditRect = .full, marks: [EditMark] = []) {
        self.rotation = rotation; self.crop = crop; self.marks = marks
    }

    /// The original, unchanged.
    public static let original = AttachmentEdit()

    /// Markup colours offered before the custom picker; the first is the default.
    public static let colors = ["#E5484D", "#F5C542", "#4CAF50", "#3B82F6", "#FFFFFF", "#000000"]
    public static let colorNames = ["#E5484D": "Red", "#F5C542": "Yellow", "#4CAF50": "Green", "#3B82F6": "Blue",
                                    "#FFFFFF": "White", "#000000": "Black"]
    /// Stroke widths and text sizes in on-screen points, converted to
    /// fractions of the picture's longer edge as they are drawn.
    public static let strokeSizes: ClosedRange<Double> = 2...40
    public static let initialStrokeSize = 6.0
    public static let textSizes: ClosedRange<Double> = 14...96
    public static let initialTextSize = 32.0
    public static let maxTextLength = 200
    /// Shown on a draft whose edits couldn't be applied; nothing is uploaded.
    public static let failureMessage = "Couldn’t apply your edits."

    private static let epsilon = 1e-6

    /// Anything differs from the original.
    public var isEdited: Bool {
        rotation != 0 || !marks.isEmpty || abs(crop.x) > Self.epsilon || abs(crop.y) > Self.epsilon
            || abs(crop.width - 1) > Self.epsilon || abs(crop.height - 1) > Self.epsilon
    }

    /// 90 and 270 swap the edges.
    public static func rotatedSize<Value>(width: Value, height: Value, rotation: Int) -> (width: Value, height: Value) {
        rotation == 90 || rotation == 270 ? (height, width) : (width, height)
    }

    /// Turns the picture a quarter anticlockwise, carrying the crop and marks with it.
    public func rotatedLeft() -> AttachmentEdit {
        AttachmentEdit(
            rotation: (rotation + 270) % 360,
            crop: EditRect(x: crop.y, y: 1 - (crop.x + crop.width), width: crop.height, height: crop.width),
            marks: marks.map { mark in
                switch mark {
                case .stroke(var stroke):
                    let points = stroke.points
                    stroke.points = points.indices.map { index in
                        index % 2 == 0 ? (index + 1 < points.count ? points[index + 1] : 0) : 1 - points[index - 1]
                    }
                    return .stroke(stroke)
                case .text(var text):
                    (text.x, text.y) = (text.y, 1 - text.x)
                    return .text(text)
                }
            })
    }

    /// The crop in pixels of the rotated `width` × `height` original. Video
    /// crops start and measure on even pixels (4:2:0 chroma); every crop keeps
    /// at least one pixel (two for video).
    public func cropPixels(width: Int, height: Int, even: Bool = false) -> PixelRect {
        let size = Self.rotatedSize(width: width, height: height, rotation: rotation)
        let rotatedWidth = Double(size.width), rotatedHeight = Double(size.height)
        let left = min(max(Self.pixel(crop.x * rotatedWidth), 0), size.width - 1)
        let top = min(max(Self.pixel(crop.y * rotatedHeight), 0), size.height - 1)
        let right = max(left + 1, min(size.width, Self.pixel((crop.x + crop.width) * rotatedWidth)))
        let bottom = max(top + 1, min(size.height, Self.pixel((crop.y + crop.height) * rotatedHeight)))
        guard even else { return PixelRect(left: left, top: top, width: right - left, height: bottom - top) }
        let evenLeft = left - left % 2
        let evenTop = top - top % 2
        let evenWidth = max(2, right - evenLeft - (right - evenLeft) % 2)
        let evenHeight = max(2, bottom - evenTop - (bottom - evenTop) % 2)
        return PixelRect(left: evenLeft, top: evenTop, width: evenWidth, height: evenHeight)
    }

    /// Rounds half away from zero (as `Math.round` for the non-negative values here).
    private static func pixel(_ value: Double) -> Int {
        guard value.isFinite else { return 0 }
        return Int(min(max(value, -1e9), 1e9).rounded())
    }

    /// Keeps a crop inside the image and at least `min` (a fraction) on each side.
    public static func clampCrop(_ crop: EditRect, min minimum: Double = 0.02) -> EditRect {
        let width = min(1, max(minimum, crop.width))
        let height = min(1, max(minimum, crop.height))
        return EditRect(x: min(1 - width, max(0, crop.x)), y: min(1 - height, max(0, crop.y)), width: width, height: height)
    }

    /// The part of the crop frame a drag holds.
    public enum CropHandle: String, CaseIterable, Sendable {
        case move, n, ne, e, se, s, sw, w, nw

        var movesLeft: Bool { self == .w || self == .nw || self == .sw }
        var movesRight: Bool { self == .e || self == .ne || self == .se }
        var movesTop: Bool { self == .n || self == .ne || self == .nw }
        var movesBottom: Bool { self == .s || self == .se || self == .sw }
    }

    /// A crop dragged by `dx`, `dy` (fractions of the image) from `start` by
    /// one of its handles, kept inside the image and at least `minWidth` ×
    /// `minHeight` (fractions).
    public static func dragCrop(_ start: EditRect, handle: CropHandle, dx: Double, dy: Double,
                                minWidth: Double, minHeight: Double) -> EditRect {
        func clamp(_ value: Double, _ low: Double, _ high: Double) -> Double { min(max(value, low), max(low, high)) }
        if handle == .move {
            return EditRect(x: clamp(start.x + dx, 0, 1 - start.width), y: clamp(start.y + dy, 0, 1 - start.height),
                            width: start.width, height: start.height)
        }
        var left = start.x, top = start.y, right = start.x + start.width, bottom = start.y + start.height
        if handle.movesLeft { left = clamp(left + dx, 0, right - minWidth) }
        if handle.movesRight { right = clamp(right + dx, left + minWidth, 1) }
        if handle.movesTop { top = clamp(top + dy, 0, bottom - minHeight) }
        if handle.movesBottom { bottom = clamp(bottom + dy, top + minHeight, 1) }
        return EditRect(x: left, y: top, width: right - left, height: bottom - top)
    }

    // MARK: Colours

    /// `#RRGGBB` as 0...1 components; nil for anything else.
    public static func components(_ hex: String) -> (red: Double, green: Double, blue: Double)? {
        let digits = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
        guard digits.count == 6, digits.allSatisfy(\.isHexDigit), let value = UInt32(digits, radix: 16) else { return nil }
        return (Double((value >> 16) & 0xFF) / 255, Double((value >> 8) & 0xFF) / 255, Double(value & 0xFF) / 255)
    }

    /// Upper-case `#RRGGBB` for 0...1 components (clamped).
    public static func hex(red: Double, green: Double, blue: Double) -> String {
        func byte(_ value: Double) -> Int { value.isFinite ? Int((min(1, max(0, value)) * 255).rounded()) : 0 }
        return String(format: "#%02X%02X%02X", byte(red), byte(green), byte(blue))
    }

    // MARK: Files

    /// Still types the editor opens where this device decodes them; callers
    /// also rule out animation (a GIF or animated WebP/PNG would keep one frame).
    public static func editableImageType(_ contentType: String) -> Bool {
        let type = contentType.split(separator: ";").first.map { $0.trimmingCharacters(in: .whitespaces).lowercased() } ?? ""
        return ["image/jpeg", "image/jpg", "image/pjpeg", "image/png", "image/webp", "image/avif", "image/heic", "image/heif",
                "image/bmp", "image/x-bmp", "image/x-ms-bmp", "image/tiff"].contains(type)
    }

    /// An edited video's crop (even pixels of its display-oriented frames,
    /// turned by the edit) and output size: the crop with its short edge
    /// capped at `maxHeight` (0: no cap), in even pixels.
    public func videoFrames(displayWidth: Int, displayHeight: Int, maxHeight: Int) -> (crop: PixelRect, width: Int, height: Int) {
        let crop = cropPixels(width: displayWidth, height: displayHeight, even: true)
        let size = AttachmentPolicy.videoOutputSize(width: crop.width, height: crop.height, maxHeight: maxHeight)
        return (crop, size.width, size.height)
    }
}

/// Fractions of the rotated, uncropped image.
public struct EditRect: Equatable, Sendable, Codable {
    public var x: Double
    public var y: Double
    public var width: Double
    public var height: Double

    public init(x: Double, y: Double, width: Double, height: Double) {
        self.x = x; self.y = y; self.width = width; self.height = height
    }

    public static let full = EditRect(x: 0, y: 0, width: 1, height: 1)
}

/// Whole pixels of the rotated image, from its top-left corner.
public struct PixelRect: Equatable, Sendable, Codable {
    public var left: Int
    public var top: Int
    public var width: Int
    public var height: Int

    public init(left: Int, top: Int, width: Int, height: Int) {
        self.left = left; self.top = top; self.width = width; self.height = height
    }
}

/// A freehand line. `width` is a fraction of the rotated image's longer edge;
/// `points` are flat x, y pairs in the image's fractions. One point is a dot.
public struct StrokeMark: Equatable, Sendable, Codable {
    public var color: String
    public var width: Double
    public var points: [Double]

    public init(color: String, width: Double, points: [Double]) {
        self.color = color; self.width = width; self.points = points
    }
}

/// One line of text centred on `x`, `y`; `size` is its font size as a
/// fraction of the rotated image's longer edge. Text stays upright when the
/// image turns.
public struct TextMark: Equatable, Sendable, Codable {
    public var color: String
    public var size: Double
    public var x: Double
    public var y: Double
    public var text: String

    public init(color: String, size: Double, x: Double, y: Double, text: String) {
        self.color = color; self.size = size; self.x = x; self.y = y; self.text = text
    }
}

/// Encoded as web's `{ "kind": "stroke" | "text", ... }`.
public enum EditMark: Equatable, Sendable, Codable {
    case stroke(StrokeMark)
    case text(TextMark)

    private enum CodingKeys: String, CodingKey { case kind }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(String.self, forKey: .kind) {
        case "stroke": self = .stroke(try StrokeMark(from: decoder))
        case "text": self = .text(try TextMark(from: decoder))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Unknown mark")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .stroke(let mark):
            try values.encode("stroke", forKey: .kind)
            try mark.encode(to: encoder)
        case .text(let mark):
            try values.encode("text", forKey: .kind)
            try mark.encode(to: encoder)
        }
    }
}

/// Where the picture sits on the editor's stage, in points.
struct AttachmentEditLayout: Equatable {
    /// The shown region (the crop, or the whole picture while cropping).
    var width: Double
    var height: Double
    /// The whole rotated picture at the same scale.
    var fullWidth: Double
    var fullHeight: Double
    /// The whole rotated picture's longer edge: stroke widths and text sizes
    /// are fractions of it, so what you see is what is sent.
    var edge: Double { max(fullWidth, fullHeight) }

    /// `region` of a rotated picture of `pictureWidth` × `pictureHeight`
    /// fitted inside `box` less `padding` on every side.
    static func fit(pictureWidth: Double, pictureHeight: Double, region: EditRect, boxWidth: Double, boxHeight: Double,
                    padding: Double) -> AttachmentEditLayout? {
        let width = region.width * pictureWidth, height = region.height * pictureHeight
        guard width > 0, height > 0, boxWidth > 0, boxHeight > 0 else { return nil }
        let scale = max(0.0001, min((boxWidth - padding * 2) / width, (boxHeight - padding * 2) / height))
        return AttachmentEditLayout(width: width * scale, height: height * scale,
                                    fullWidth: pictureWidth * scale, fullHeight: pictureHeight * scale)
    }

    /// A point in the shown region as fractions of the whole rotated picture.
    func fraction(x: Double, y: Double, region: EditRect) -> (x: Double, y: Double) {
        (region.x + x / width * region.width, region.y + y / height * region.height)
    }

    /// Fractions of the whole rotated picture as a point in the shown region.
    func point(x: Double, y: Double, region: EditRect) -> (x: Double, y: Double) {
        ((x - region.x) / region.width * width, (y - region.y) / region.height * height)
    }

    /// A stroke drawn now at `points` on screen, as a fraction of the longer edge.
    func strokeWidth(points: Double) -> Double { points / max(edge, 0.0001) }

    /// The brush ring's diameter in Draw: exactly how wide a stroke drawn now
    /// at `points` paints on screen, and never under `minimumBrushRing`.
    func brushDiameter(points: Double) -> Double { max(Self.minimumBrushRing, strokeWidth(points: points) * edge) }

    static let minimumBrushRing = 6.0
}

/// The editor's tools; the pencil opens Draw, and a staged file's preview
/// can open any of them.
enum AttachmentEditTool: String, CaseIterable, Sendable {
    case crop, draw, text

    /// The pencil's.
    static let initial = AttachmentEditTool.draw

    var title: String {
        switch self {
        case .crop: return "Crop"
        case .draw: return "Draw"
        case .text: return "Text"
        }
    }

    /// Lucide, as on web.
    var icon: String {
        switch self {
        case .crop: return "crop"
        case .draw: return "brush"
        case .text: return "type"
        }
    }
}

/// Where Draw's brush ring stands in for a hovering pointer (mouse,
/// trackpad, iPad pointer, stylus hover) over the picture. It sits at a
/// stroke's tip while that pointer draws; touches never show it.
struct AttachmentEditBrushPointer: Equatable {
    /// The ring's centre in the picture's points; nil hides it.
    private(set) var location: CGPoint?
    private(set) var hovering = false
    /// The stroke being drawn follows a hovering pointer.
    private(set) var drawing = false
    private var lastHoverEnd: Date?
    /// iPadOS can end a pointer's hover just before its click arrives as a press.
    static let pressGrace: TimeInterval = 0.3

    init() {}

    mutating func hovered(at point: CGPoint) {
        hovering = true
        location = point
    }

    mutating func hoverEnded(now: Date = Date()) {
        hovering = false
        lastHoverEnd = now
        if !drawing { location = nil }
    }

    /// A press moved to `point`; `starting` is its first report.
    mutating func pressed(at point: CGPoint, starting: Bool, now: Date = Date()) {
        if starting { drawing = hovering || lastHoverEnd.map { now.timeIntervalSince($0) < Self.pressGrace } == true }
        if drawing { location = point }
    }

    mutating func released() {
        drawing = false
        if !hovering { location = nil }
    }
}

/// An open text field over the picture: a new mark, or `index` being changed.
struct AttachmentEditTextField: Equatable {
    var index: Int?
    var x: Double
    var y: Double
    var value: String
    var color: String
    /// Points on screen; a fraction of the picture's longer edge once committed.
    var points: Double

    /// `edit` with this field committed: a new mark, the changed one, or
    /// without it when emptied. Unchanged when nothing differs.
    func applied(to edit: AttachmentEdit, edge: Double) -> AttachmentEdit {
        let text = String(value.trimmingCharacters(in: .whitespacesAndNewlines).prefix(AttachmentEdit.maxTextLength))
        let size = points / max(edge, 0.0001)
        var marks = edit.marks
        if let index {
            guard marks.indices.contains(index), case .text(var mark) = marks[index] else { return edit }
            if text.isEmpty {
                marks.remove(at: index)
            } else if text != mark.text || color != mark.color || abs(size - mark.size) > 1e-9 {
                mark.text = text; mark.color = color; mark.size = size
                marks[index] = .text(mark)
            } else {
                return edit
            }
        } else if !text.isEmpty {
            marks.append(.text(TextMark(color: color, size: size, x: x, y: y, text: text)))
        } else {
            return edit
        }
        var next = edit
        next.marks = marks
        return next
    }
}

/// The editor's undo history: every committed change is one step.
struct AttachmentEditHistory: Equatable {
    private(set) var states: [AttachmentEdit]
    private(set) var index = 0

    init(_ initial: AttachmentEdit) { states = [initial] }

    var current: AttachmentEdit { states[index] }
    var canUndo: Bool { index > 0 }
    var canRedo: Bool { index < states.count - 1 }

    /// Adds `next` after the current state, dropping anything undone. Nothing for no change.
    mutating func commit(_ next: AttachmentEdit) {
        guard next != current else { return }
        states = Array(states[...index]) + [next]
        index += 1
    }

    mutating func undo() { if canUndo { index -= 1 } }
    mutating func redo() { if canRedo { index += 1 } }
}
