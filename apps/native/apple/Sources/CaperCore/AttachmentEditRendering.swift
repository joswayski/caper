import CoreGraphics
import CoreImage
import CoreImage.CIFilterBuiltins
import CoreText
import Foundation
import UniformTypeIdentifiers

/// Paints attachment edits with CoreGraphics and CoreText, the same way for
/// the upload, the composer thumbnail, a video's overlay and the editor's
/// stage (docs/media.md, "Editing attachments before sending"): the upright
/// original turned by the rotation, the marks painted in the rotated image's
/// space (round-capped strokes, upright Satoshi Bold text), then the crop.
enum AttachmentEditRenderer {
    /// Registered by `CaperFontLoader`; CoreText falls back to the system font
    /// where it is missing (unit tests).
    static let fontName = "Satoshi-Bold" as CFString

    /// The edited `region` (pixels of the rotated image) of the upright
    /// `source` of `width` × `height` pixels, scaled to `outputWidth` ×
    /// `outputHeight`. Without a source only the marks are painted, on
    /// transparency (a video's overlay).
    static func render(_ source: CGImage?, width: Int, height: Int, edit: AttachmentEdit, region: PixelRect,
                       outputWidth: Int, outputHeight: Int, space: CGColorSpace) -> CGImage? {
        guard width > 0, height > 0, outputWidth > 0, outputHeight > 0, region.width > 0, region.height > 0,
              let context = CGContext(data: nil, width: outputWidth, height: outputHeight, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        context.interpolationQuality = .high
        // Top-left origin with y down, like the other clients' canvases.
        context.translateBy(x: 0, y: CGFloat(outputHeight))
        context.scaleBy(x: 1, y: -1)
        context.scaleBy(x: CGFloat(outputWidth) / CGFloat(region.width), y: CGFloat(outputHeight) / CGFloat(region.height))
        context.translateBy(x: -CGFloat(region.left), y: -CGFloat(region.top))
        if let source { drawRotated(context, source, width: CGFloat(width), height: CGFloat(height), rotation: edit.rotation) }
        let rotated = AttachmentEdit.rotatedSize(width: CGFloat(width), height: CGFloat(height), rotation: edit.rotation)
        paintMarks(context, edit.marks, width: rotated.width, height: rotated.height)
        return context.makeImage()
    }

    /// A small render of `edit` from the editor's picture, staged as a PNG for
    /// the composer's thumbnail; nil when it can't be drawn.
    static func stagePreview(_ picture: CGImage, edit: AttachmentEdit, maxEdge: Int = 640) -> URL? {
        let region = edit.cropPixels(width: picture.width, height: picture.height)
        let size = AttachmentPolicy.fitWithin(width: region.width, height: region.height, edge: maxEdge)
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
              let image = render(picture, width: picture.width, height: picture.height, edit: edit, region: region,
                                 outputWidth: size.width, outputHeight: size.height, space: space),
              let data = AttachmentPreparer.encode(image, type: UTType.png.identifier, quality: 1) else { return nil }
        return AttachmentPreparer.writeStaged(data, name: "edit-preview.png")
    }

    /// Draws the upright original turned clockwise by `rotation` into the
    /// rotated image's top-down space. Exact quarter-turn matrices keep
    /// lossless pixels exact.
    static func drawRotated(_ context: CGContext, _ source: CGImage, width: CGFloat, height: CGFloat, rotation: Int) {
        context.saveGState()
        switch rotation {
        case 90: context.concatenate(CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: height, ty: 0))
        case 180: context.concatenate(CGAffineTransform(a: -1, b: 0, c: 0, d: -1, tx: width, ty: height))
        case 270: context.concatenate(CGAffineTransform(a: 0, b: -1, c: 1, d: 0, tx: 0, ty: width))
        default: break
        }
        // CoreGraphics draws images bottom-up: flip locally so the top row lands at y = 0.
        context.translateBy(x: 0, y: height)
        context.scaleBy(x: 1, y: -1)
        context.draw(source, in: CGRect(x: 0, y: 0, width: width, height: height))
        context.restoreGState()
    }

    /// Paints marks in a top-down space of the rotated image's `width` ×
    /// `height` (pixels, or points on the editor's stage).
    static func paintMarks(_ context: CGContext, _ marks: [EditMark], width: CGFloat, height: CGFloat) {
        let edge = max(width, height)
        for mark in marks {
            switch mark {
            case .stroke(let stroke):
                guard stroke.points.count >= 2 else { continue }
                let color = cgColor(stroke.color)
                let lineWidth = CGFloat(stroke.width) * edge
                let points = stride(from: 0, to: stroke.points.count - 1, by: 2).map { index in
                    CGPoint(x: CGFloat(stroke.points[index]) * width, y: CGFloat(stroke.points[index + 1]) * height)
                }
                guard points.count > 1 else {
                    // A tap is a dot as wide as the line.
                    context.setFillColor(color)
                    context.fillEllipse(in: CGRect(x: points[0].x - lineWidth / 2, y: points[0].y - lineWidth / 2,
                                                   width: lineWidth, height: lineWidth))
                    continue
                }
                context.setStrokeColor(color)
                context.setLineWidth(lineWidth)
                context.setLineCap(.round)
                context.setLineJoin(.round)
                context.beginPath()
                context.addLines(between: points)
                context.strokePath()
            case .text(let text):
                guard !text.text.isEmpty else { continue }
                drawText(context, text.text, color: cgColor(text.color), size: CGFloat(text.size) * edge,
                         centre: CGPoint(x: CGFloat(text.x) * width, y: CGFloat(text.y) * height))
            }
        }
    }

    /// One line centred on `centre`: horizontally on its advance, vertically
    /// on the middle of its ascent and descent.
    static func drawText(_ context: CGContext, _ text: String, color: CGColor, size: CGFloat, centre: CGPoint) {
        guard size > 0 else { return }
        let line = textLine(text, size: size, color: color)
        var ascent: CGFloat = 0, descent: CGFloat = 0, leading: CGFloat = 0
        let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
        context.saveGState()
        // CoreText draws bottom-up: flip about the baseline.
        context.translateBy(x: centre.x - width / 2, y: centre.y + (ascent - descent) / 2)
        context.scaleBy(x: 1, y: -1)
        context.textMatrix = .identity
        context.textPosition = .zero
        CTLineDraw(line, context)
        context.restoreGState()
    }

    /// A line's advance width at `size`, for the editor's hit testing.
    static func textWidth(_ text: String, size: CGFloat) -> CGFloat {
        guard size > 0, !text.isEmpty else { return 0 }
        return CGFloat(CTLineGetTypographicBounds(textLine(text, size: size, color: nil), nil, nil, nil))
    }

    static func textLine(_ text: String, size: CGFloat, color: CGColor?) -> CTLine {
        var attributes: [NSAttributedString.Key: Any] = [
            NSAttributedString.Key(kCTFontAttributeName as String): CTFontCreateWithName(fontName, size, nil),
        ]
        if let color { attributes[NSAttributedString.Key(kCTForegroundColorAttributeName as String)] = color }
        return CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: attributes) as CFAttributedString)
    }

    /// `#RRGGBB` in sRGB; unreadable values paint black.
    static func cgColor(_ hex: String) -> CGColor {
        let rgb = AttachmentEdit.components(hex) ?? (red: 0, green: 0, blue: 0)
        return CGColor(srgbRed: CGFloat(rgb.red), green: CGFloat(rgb.green), blue: CGFloat(rgb.blue), alpha: 1)
    }

    // MARK: Video frames

    /// One display-oriented video `frame` turned by `rotation`, cropped to
    /// `crop` (top-down pixels of the turned frame, measured on `display`),
    /// scaled to `width` × `height`, with `overlay` (the marks, already at
    /// the output size) on top. Nil when the frame isn't the size the crop
    /// was measured on, so a mismatch fails instead of cropping the wrong part.
    static func editedFrame(_ frame: CIImage, rotation: Int, crop: PixelRect, display: (width: Int, height: Int),
                            width: Int, height: Int, overlay: CIImage?) -> CIImage? {
        let extent = frame.extent
        guard width > 0, height > 0, crop.width > 0, crop.height > 0, extent.width > 0, extent.height > 0,
              abs(extent.width - CGFloat(display.width)) <= 2, abs(extent.height - CGFloat(display.height)) <= 2 else { return nil }
        let frameWidth = extent.width, frameHeight = extent.height
        var image = frame.transformed(by: CGAffineTransform(translationX: -extent.minX, y: -extent.minY))
        // Core Image is bottom-up, so these are the top-down turns mirrored.
        switch rotation {
        case 90: image = image.transformed(by: CGAffineTransform(a: 0, b: -1, c: 1, d: 0, tx: 0, ty: frameWidth))
        case 180: image = image.transformed(by: CGAffineTransform(a: -1, b: 0, c: 0, d: -1, tx: frameWidth, ty: frameHeight))
        case 270: image = image.transformed(by: CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: frameHeight, ty: 0))
        default: break
        }
        let turned = AttachmentEdit.rotatedSize(width: frameWidth, height: frameHeight, rotation: rotation)
        let nominal = AttachmentEdit.rotatedSize(width: CGFloat(display.width), height: CGFloat(display.height), rotation: rotation)
        let scaleToFrameX = turned.width / nominal.width, scaleToFrameY = turned.height / nominal.height
        let rect = CGRect(x: CGFloat(crop.left) * scaleToFrameX, y: turned.height - CGFloat(crop.top + crop.height) * scaleToFrameY,
                          width: CGFloat(crop.width) * scaleToFrameX, height: CGFloat(crop.height) * scaleToFrameY)
        image = image.cropped(to: rect).transformed(by: CGAffineTransform(translationX: -rect.minX, y: -rect.minY))
        let scaleX = CGFloat(width) / rect.width, scaleY = CGFloat(height) / rect.height
        if abs(scaleX - 1) > 1e-6 || abs(scaleY - 1) > 1e-6 {
            // Lanczos, from clamped edges so the border doesn't fade.
            let filter = CIFilter.lanczosScaleTransform()
            filter.inputImage = image.clampedToExtent()
            filter.scale = Float(scaleY)
            filter.aspectRatio = Float(scaleX / scaleY)
            guard let scaled = filter.outputImage else { return nil }
            image = scaled
        }
        image = image.cropped(to: CGRect(x: 0, y: 0, width: width, height: height))
        if let overlay { image = overlay.composited(over: image) }
        return image
    }
}
