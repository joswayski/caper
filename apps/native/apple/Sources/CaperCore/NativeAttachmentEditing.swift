import AVFoundation
import CoreGraphics
import CoreImage
import Observation

/// Draft-only editing shared by the AppKit and UIKit canvases. Coordinates,
/// history and upload rendering keep the existing attachment-edit contract.
@MainActor @Observable final class NativeAttachmentEditing {
    let request: AttachmentEditRequest
    let close: () -> Void
    var picture: AttachmentEditorPicture?
    var failed = false
    var history: AttachmentEditHistory
    var tool: AttachmentEditTool?
    var color = AttachmentEdit.colors[0]
    var strokePoints = AttachmentEdit.initialStrokeSize
    var textPoints = AttachmentEdit.initialTextSize
    var erasing = false
    var field: AttachmentEditTextField?
    private var fieldEdge = 1.0
    var brush = AttachmentEditBrushPointer()
    var scale: CGFloat = 1
    var offset: CGSize = .zero
    private var liveStroke: StrokeMark?
    private var erased: Set<Int> = []
    private var liveCrop: EditRect?
    private var liveText: TextMark?
    private var textIndex: Int?
    private var start: CGPoint?
    private var last: CGPoint?
    private var cropHandle: AttachmentEdit.CropHandle?
    private var fieldClosed = false
    private var moved = false
    private var saving = false
    @ObservationIgnored private var blurred: (edit: AttachmentEdit, image: CGImage)?

    init(request: AttachmentEditRequest, close: @escaping () -> Void) {
        self.request = request; self.close = close
        history = AttachmentEditHistory(request.edit); tool = request.tool
    }
    var spoilered: Bool { request.spoiler?.isOn() ?? false }
    var covered: Bool { tool == nil && spoilered }
    var playback: URL? { tool == nil && !spoilered && history.current == request.edit ? request.playback : nil }
    var region: EditRect { tool == .crop ? .full : history.current.crop }
    var shown: AttachmentEdit {
        var edit = history.current
        if let liveText, let textIndex { edit.marks[textIndex] = .text(liveText) }
        edit.marks = edit.marks.enumerated().filter { !erased.contains($0.offset) && $0.offset != field?.index }.map(\.element)
        if let liveStroke { edit.marks.append(.stroke(liveStroke)) }
        if let liveCrop { edit.crop = liveCrop }
        return edit
    }
    func load() async {
        let loaded = await AttachmentEditorPicture.load(request.original, kind: request.kind, maxEdge: request.pictureMode ? 4096 : 2048)
        guard !Task.isCancelled else { return }
        picture = loaded; failed = loaded == nil; blurred = nil
    }
    func layout(in size: CGSize) -> AttachmentEditLayout? {
        guard let image = picture?.image else { return nil }
        let rotated = AttachmentEdit.rotatedSize(width: Double(image.width), height: Double(image.height), rotation: history.current.rotation)
        return AttachmentEditLayout.fit(pictureWidth: rotated.width, pictureHeight: rotated.height, region: region,
            boxWidth: size.width, boxHeight: size.height, padding: 28)
    }
    func rect(in size: CGSize) -> CGRect {
        guard let layout = layout(in: size) else { return .zero }
        return CGRect(x: (size.width - layout.width) / 2, y: (size.height - layout.height) / 2, width: layout.width, height: layout.height)
    }
    func choose(_ next: AttachmentEditTool) {
        commitField(); cancelGesture()
        tool = AttachmentEditTool.picking(next, from: tool, pictureMode: request.pictureMode)
        erasing = false; scale = 1; offset = .zero
    }
    func chooseColor(_ next: String) { color = next; erasing = false; field?.color = next }
    func chooseSize(_ points: Double) {
        if tool == .draw { strokePoints = points } else { textPoints = points; field?.points = points }
    }
    func sizeFromSlider(_ points: Double, dragging: Bool) {
        let range = tool == .draw ? AttachmentEdit.strokeSizes : AttachmentEdit.textSizes
        let normal = tool == .draw ? AttachmentEdit.initialStrokeSize : AttachmentEdit.initialTextSize
        chooseSize(dragging && abs(points - normal) <= max(1, (range.upperBound - range.lowerBound) * 0.02) ? normal : points.rounded())
    }
    func undo() { field = nil; cancelGesture(); history.undo() }
    func redo() { field = nil; cancelGesture(); history.redo() }
    func reset() { field = nil; cancelGesture(); history.commit(request.pictureMode ? request.edit : .original) }
    func rotate() { commitField(); cancelGesture(); history.commit(history.current.rotatedLeft()) }
    func escape() { if field != nil { field = nil } else { close() } }
    func remove() { request.remove?(); close() }
    func commitField() {
        guard let field else { return }
        history.commit(field.applied(to: history.current, edge: fieldEdge)); self.field = nil
    }
    func setText(_ value: String) { field?.value = String(value.replacingOccurrences(of: "\n", with: " ").prefix(AttachmentEdit.maxTextLength)) }
    func done() {
        guard !saving else { return }
        commitField()
        let edit = history.current
        guard request.pictureMode || edit != request.edit else { close(); return }
        guard let picture else { return }
        saving = true
        request.save(edit, !request.pictureMode && edit.isEdited ? AttachmentEditRenderer.stagePreview(picture.image, edit: edit) : nil)
        close()
    }
    var cropSize: Double {
        guard let image = picture?.image else { return 1 }
        let size = AttachmentEdit.rotatedSize(width: Double(image.width), height: Double(image.height), rotation: history.current.rotation)
        return history.current.crop.width * size.width / min(size.width, size.height)
    }
    func resizeCrop(_ fraction: Double) {
        guard let image = picture?.image else { return }
        let size = AttachmentEdit.rotatedSize(width: Double(image.width), height: Double(image.height), rotation: history.current.rotation)
        let crop = history.current.crop, side = fraction * min(size.width, size.height)
        var next = history.current
        next.crop = AttachmentEdit.clampCrop(EditRect(x: crop.x + crop.width / 2 - side / size.width / 2,
            y: crop.y + crop.height / 2 - side / size.height / 2, width: side / size.width, height: side / size.height), min: 0)
        history.commit(next)
    }
    /// Points passed by either platform are in the unscaled, top-down stage.
    func begin(at point: CGPoint, size: CGSize) {
        guard let layout = layout(in: size), tool != nil else { return }
        let rect = rect(in: size), local = CGPoint(x: point.x - rect.minX, y: point.y - rect.minY)
        guard rect.insetBy(dx: tool == .crop ? -22 : 0, dy: tool == .crop ? -22 : 0).contains(point) else { return }
        start = local; last = nil; moved = false; fieldClosed = false
        if tool == .text {
            if field != nil { commitField(); fieldClosed = true; return }
            let fraction = layout.fraction(x: local.x, y: local.y, region: region)
            textIndex = history.current.marks.indices.reversed().first {
                guard case .text(let mark) = history.current.marks[$0] else { return false }
                let points = mark.size * layout.edge
                return abs((fraction.x - mark.x) * layout.fullWidth) <= Double(AttachmentEditRenderer.textWidth(mark.text, size: points)) / 2 + 8
                    && abs((fraction.y - mark.y) * layout.fullHeight) <= points * 0.6 + 8
            }
        } else if tool == .crop {
            let crop = history.current.crop
            let frame = CGRect(x: crop.x * layout.width, y: crop.y * layout.height, width: crop.width * layout.width, height: crop.height * layout.height)
            cropHandle = request.pictureMode ? nil : AttachmentEdit.CropHandle.allCases.filter { $0 != .move }.min {
                distance(local, Self.handle($0, in: frame)) < distance(local, Self.handle($1, in: frame))
            }
            if let handle = cropHandle, distance(local, Self.handle(handle, in: frame)) > 22 { cropHandle = nil }
            if cropHandle == nil && frame.contains(local) { cropHandle = .move }
        }
        drag(to: point, size: size)
    }
    func drag(to point: CGPoint, size: CGSize) {
        guard let start, let layout = layout(in: size), !fieldClosed else { return }
        let rect = rect(in: size), local = CGPoint(x: point.x - rect.minX, y: point.y - rect.minY)
        let fraction = layout.fraction(x: local.x, y: local.y, region: region)
        switch tool {
        case .draw:
            brush.pressed(at: local, starting: last == nil)
            if erasing {
                let previous = last ?? local, radius = layout.brushDiameter(points: strokePoints) / 2
                let steps = max(1, Int((distance(previous, local) / max(1, radius / 2)).rounded(.up)))
                for step in 1...steps {
                    let t = Double(step) / Double(steps)
                    let p = layout.fraction(x: previous.x + (local.x - previous.x) * t, y: previous.y + (local.y - previous.y) * t, region: region)
                    erased.formUnion(AttachmentEdit.strokesTouched(history.current.marks, x: p.x, y: p.y, radius: radius, width: layout.fullWidth, height: layout.fullHeight))
                }
            } else if liveStroke == nil {
                liveStroke = StrokeMark(color: color, width: layout.strokeWidth(points: strokePoints), points: [fraction.x, fraction.y])
            } else if let last, distance(last, local) >= 1.5 { liveStroke?.points += [fraction.x, fraction.y] }
            last = local
        case .crop:
            guard let cropHandle else { return }
            liveCrop = AttachmentEdit.dragCrop(history.current.crop, handle: cropHandle,
                dx: (local.x - start.x) / layout.width, dy: (local.y - start.y) / layout.height,
                minWidth: min(1, max(0.02, 32 / layout.width)), minHeight: min(1, max(0.02, 32 / layout.height)))
        case .text:
            guard let textIndex, case .text(var mark) = history.current.marks[textIndex], distance(start, local) > 4 || moved else { return }
            moved = true
            let origin = layout.fraction(x: start.x, y: start.y, region: region)
            mark.x = min(1, max(0, mark.x + fraction.x - origin.x)); mark.y = min(1, max(0, mark.y + fraction.y - origin.y))
            liveText = mark
        case nil: break
        }
    }
    func end(size: CGSize) {
        defer { cancelGesture() }
        guard let start, let layout = layout(in: size), !fieldClosed else { return }
        var next = history.current
        if let liveStroke { next.marks.append(.stroke(liveStroke)) }
        if !erased.isEmpty { next.marks = next.marks.enumerated().filter { !erased.contains($0.offset) }.map(\.element) }
        if let liveCrop { next.crop = AttachmentEdit.clampCrop(liveCrop, min: request.pictureMode ? 0 : 0.02) }
        if let liveText, let textIndex { next.marks[textIndex] = .text(liveText) }
        history.commit(next)
        if tool == .text && !moved {
            fieldEdge = layout.edge
            if let textIndex, case .text(let mark) = next.marks[textIndex] {
                color = mark.color; textPoints = min(96, max(14, (mark.size * layout.edge).rounded()))
                field = AttachmentEditTextField(index: textIndex, x: mark.x, y: mark.y, value: mark.text, color: mark.color, points: mark.size * layout.edge)
            } else {
                let p = layout.fraction(x: start.x, y: start.y, region: region)
                field = AttachmentEditTextField(index: nil, x: p.x, y: p.y, value: "", color: color, points: textPoints)
            }
        }
    }
    func cancelGesture() {
        start = nil; last = nil; liveStroke = nil; liveCrop = nil; liveText = nil; textIndex = nil; erased = []
        brush.released()
    }
    static func handle(_ handle: AttachmentEdit.CropHandle, in rect: CGRect) -> CGPoint {
        CGPoint(x: handle.movesLeft ? rect.minX : handle.movesRight ? rect.maxX : rect.midX,
                y: handle.movesTop ? rect.minY : handle.movesBottom ? rect.maxY : rect.midY)
    }
    private func distance(_ a: CGPoint, _ b: CGPoint) -> Double { hypot(a.x - b.x, a.y - b.y) }

    func paintSizePreview(in context: CGContext, size: CGSize) {
        if tool == .draw {
            let diameter = min(strokePoints, 40)
            let rect = CGRect(x: (size.width - diameter) / 2, y: (size.height - diameter) / 2, width: diameter, height: diameter)
            if !erasing { context.setFillColor(AttachmentEditRenderer.cgColor(color)); context.fillEllipse(in: rect) }
            context.setStrokeColor(CGColor(gray: 1, alpha: erasing ? 0.5 : 0.3)); context.setLineWidth(1.5); context.strokeEllipse(in: rect)
        } else {
            AttachmentEditRenderer.drawText(context, "Aa", color: AttachmentEditRenderer.cgColor(color), size: min(textPoints, 36),
                                           centre: CGPoint(x: size.width / 2, y: size.height / 2))
        }
    }

    func paint(in context: CGContext, size: CGSize) {
        guard let picture, let layout = layout(in: size) else { return }
        let rect = rect(in: size)
        context.saveGState()
        context.translateBy(x: size.width / 2 + offset.width, y: size.height / 2 + offset.height)
        context.scaleBy(x: scale, y: scale)
        context.translateBy(x: -size.width / 2, y: -size.height / 2)
        if covered {
            if blurred?.edit != history.current {
                blurred = nil
                let crop = history.current.cropPixels(width: picture.image.width, height: picture.image.height)
                let output = AttachmentPolicy.fitWithin(width: crop.width, height: crop.height, edge: 320)
                if let space = CGColorSpace(name: CGColorSpace.sRGB),
                   let rendered = AttachmentEditRenderer.render(picture.image, width: picture.image.width, height: picture.image.height,
                    edit: history.current, region: crop, outputWidth: output.width, outputHeight: output.height, space: space) {
                    let input = CIImage(cgImage: rendered)
                    let hidden = input.clampedToExtent().applyingFilter("CIGaussianBlur", parameters: [kCIInputRadiusKey: 24])
                    if let image = CIContext().createCGImage(hidden, from: input.extent) { blurred = (history.current, image) }
                }
            }
            if let image = blurred?.image {
                context.saveGState(); context.translateBy(x: rect.minX, y: rect.maxY); context.scaleBy(x: 1, y: -1)
                context.draw(image, in: CGRect(origin: .zero, size: rect.size)); context.restoreGState()
            }
            context.setFillColor(CGColor(gray: 0, alpha: blurred == nil ? 1 : 0.35)); context.fill(rect)
            AttachmentEditRenderer.drawText(context, "SPOILER", color: CGColor(gray: 1, alpha: 1), size: 14,
                                           centre: CGPoint(x: rect.midX, y: rect.midY))
        } else {
            context.saveGState(); context.clip(to: rect)
            context.translateBy(x: rect.minX - region.x * layout.fullWidth, y: rect.minY - region.y * layout.fullHeight)
            let rotated = AttachmentEdit.rotatedSize(width: Double(picture.image.width), height: Double(picture.image.height), rotation: shown.rotation)
            context.saveGState(); context.scaleBy(x: layout.fullWidth / rotated.width, y: layout.fullHeight / rotated.height)
            AttachmentEditRenderer.drawRotated(context, picture.image, width: CGFloat(picture.image.width), height: CGFloat(picture.image.height), rotation: shown.rotation)
            context.restoreGState()
            AttachmentEditRenderer.paintMarks(context, shown.marks, width: layout.fullWidth, height: layout.fullHeight)
            context.restoreGState()
        }
        if tool == .crop {
            let crop = shown.crop
            let frame = CGRect(x: rect.minX + crop.x * layout.width, y: rect.minY + crop.y * layout.height,
                               width: crop.width * layout.width, height: crop.height * layout.height)
            context.addRect(rect); context.addRect(frame); context.setFillColor(CGColor(gray: 0, alpha: 0.55)); context.drawPath(using: .eoFill)
            context.setStrokeColor(CGColor(gray: 1, alpha: 0.9)); context.setLineWidth(1); context.stroke(frame)
            if !request.pictureMode {
                context.setFillColor(CGColor(gray: 1, alpha: 1))
                for handle in AttachmentEdit.CropHandle.allCases where handle != .move {
                    let point = Self.handle(handle, in: frame)
                    context.fillEllipse(in: CGRect(x: point.x - 4, y: point.y - 4, width: 8, height: 8))
                }
            }
        }
        if tool == .draw, let point = brush.location {
            let diameter = layout.brushDiameter(points: strokePoints)
            let ring = CGRect(x: rect.minX + point.x - diameter / 2, y: rect.minY + point.y - diameter / 2, width: diameter, height: diameter)
            let fill = erasing ? CGColor(gray: 1, alpha: 0.3) : AttachmentEditRenderer.cgColor(color).copy(alpha: 0.3)!
            context.setFillColor(fill); context.fillEllipse(in: ring)
            context.setStrokeColor(CGColor(gray: 0, alpha: 0.7)); context.setLineWidth(3.5); context.strokeEllipse(in: ring)
            context.setStrokeColor(CGColor(gray: 1, alpha: 1)); context.setLineWidth(1.5); context.strokeEllipse(in: ring)
        }
        context.restoreGState()
    }
}
