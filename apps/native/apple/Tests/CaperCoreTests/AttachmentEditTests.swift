import CoreGraphics
import XCTest
@testable import CaperCore

/// Attachment edit geometry: every case in shared/messages/edit-cases.json,
/// as web and the other native clients run them (apps/web/src/server/edit.test.ts).
final class AttachmentEditTests: XCTestCase {
    private struct Cases: Decodable {
        struct Rotation: Decodable {
            let note: String
            let before: AttachmentEdit
            let after: AttachmentEdit
        }
        struct Crop: Decodable {
            let note: String
            let width: Int
            let height: Int
            let rotation: Int
            let crop: EditRect
            let even: Bool
            let expected: PixelRect
        }
        struct Edited: Decodable {
            let note: String
            let edit: AttachmentEdit
            let edited: Bool
        }
        let rotateLeft: [Rotation]
        let cropPixels: [Crop]
        let edited: [Edited]
    }

    private func cases() throws -> Cases {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let url = root.appendingPathComponent("shared/messages/edit-cases.json")
        return try JSONDecoder().decode(Cases.self, from: Data(contentsOf: url))
    }

    private func assertClose(_ actual: AttachmentEdit, _ expected: AttachmentEdit, _ note: String,
                             file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(actual.rotation, expected.rotation, note, file: file, line: line)
        for (value, wanted) in [(actual.crop.x, expected.crop.x), (actual.crop.y, expected.crop.y),
                                (actual.crop.width, expected.crop.width), (actual.crop.height, expected.crop.height)] {
            XCTAssertEqual(value, wanted, accuracy: 1e-9, note, file: file, line: line)
        }
        XCTAssertEqual(actual.marks.count, expected.marks.count, note, file: file, line: line)
        for (mark, wanted) in zip(actual.marks, expected.marks) {
            switch (mark, wanted) {
            case (.stroke(let stroke), .stroke(let other)):
                XCTAssertEqual(stroke.color, other.color, note, file: file, line: line)
                XCTAssertEqual(stroke.width, other.width, accuracy: 1e-9, note, file: file, line: line)
                XCTAssertEqual(stroke.points.count, other.points.count, note, file: file, line: line)
                for (point, wantedPoint) in zip(stroke.points, other.points) {
                    XCTAssertEqual(point, wantedPoint, accuracy: 1e-9, note, file: file, line: line)
                }
            case (.text(let text), .text(let other)):
                XCTAssertEqual(text.color, other.color, note, file: file, line: line)
                XCTAssertEqual(text.text, other.text, note, file: file, line: line)
                XCTAssertEqual(text.size, other.size, accuracy: 1e-9, note, file: file, line: line)
                XCTAssertEqual(text.x, other.x, accuracy: 1e-9, note, file: file, line: line)
                XCTAssertEqual(text.y, other.y, accuracy: 1e-9, note, file: file, line: line)
            default:
                XCTFail("\(note): mark kinds differ", file: file, line: line)
            }
        }
    }

    func testSharedRotateLeftCases() throws {
        let cases = try cases().rotateLeft
        XCTAssertGreaterThanOrEqual(cases.count, 4, "edit-cases.json lost its rotateLeft cases")
        for item in cases { assertClose(item.before.rotatedLeft(), item.after, item.note) }
    }

    func testSharedCropPixelCases() throws {
        let cases = try cases().cropPixels
        XCTAssertGreaterThanOrEqual(cases.count, 7, "edit-cases.json lost its cropPixels cases")
        for item in cases {
            let edit = AttachmentEdit(rotation: item.rotation, crop: item.crop)
            XCTAssertEqual(edit.cropPixels(width: item.width, height: item.height, even: item.even), item.expected, item.note)
        }
    }

    func testSharedEditedCases() throws {
        let cases = try cases().edited
        XCTAssertGreaterThanOrEqual(cases.count, 4, "edit-cases.json lost its edited cases")
        for item in cases { XCTAssertEqual(item.edit.isEdited, item.edited, item.note) }
        XCTAssertFalse(AttachmentEdit.original.isEdited)
        XCTAssertFalse(AttachmentEdit(crop: EditRect(x: 1e-7, y: 0, width: 1 - 1e-7, height: 1)).isEdited, "within 1e-6 is the original")
    }

    func testFourLeftTurnsReturnToTheStart() {
        let edit = AttachmentEdit(rotation: 0, crop: EditRect(x: 0.1, y: 0.2, width: 0.5, height: 0.3), marks: [
            .stroke(StrokeMark(color: "#000000", width: 0.01, points: [0.1, 0.2, 0.3, 0.4])),
            .text(TextMark(color: "#FFFFFF", size: 0.05, x: 0.25, y: 0.75, text: "Look")),
        ])
        assertClose(edit.rotatedLeft().rotatedLeft().rotatedLeft().rotatedLeft(), edit, "a full turn")
    }

    func testEditsRoundTripThroughWebJSON() throws {
        let json = ##"{"rotation":90,"crop":{"x":0,"y":0,"width":1,"height":1},"marks":[{"kind":"stroke","color":"#E5484D","width":0.01,"points":[0.5,0.5]},{"kind":"text","color":"#FFFFFF","size":0.05,"x":0.2,"y":0.3,"text":"Hi"}]}"##
        let edit = try JSONDecoder().decode(AttachmentEdit.self, from: Data(json.utf8))
        XCTAssertEqual(edit.marks, [
            .stroke(StrokeMark(color: "#E5484D", width: 0.01, points: [0.5, 0.5])),
            .text(TextMark(color: "#FFFFFF", size: 0.05, x: 0.2, y: 0.3, text: "Hi")),
        ])
        XCTAssertEqual(try JSONDecoder().decode(AttachmentEdit.self, from: JSONEncoder().encode(edit)), edit)
        XCTAssertThrowsError(try JSONDecoder().decode(EditMark.self, from: Data(#"{"kind":"sticker"}"#.utf8)))
    }

    func testCropsStayInsideAndKeepTheirMinimum() {
        XCTAssertEqual(AttachmentEdit.clampCrop(EditRect(x: -0.2, y: 0.9, width: 0.5, height: 0.5)), EditRect(x: 0, y: 0.5, width: 0.5, height: 0.5))
        XCTAssertEqual(AttachmentEdit.clampCrop(EditRect(x: 0.5, y: 0.5, width: 0, height: 2)), EditRect(x: 0.5, y: 0, width: 0.02, height: 1))
        let start = EditRect(x: 0.2, y: 0.2, width: 0.5, height: 0.5)
        XCTAssertEqual(AttachmentEdit.dragCrop(start, handle: .move, dx: 0.5, dy: -0.5, minWidth: 0.02, minHeight: 0.02),
                       EditRect(x: 0.5, y: 0, width: 0.5, height: 0.5), "moving stops at the edges")
        let corner = AttachmentEdit.dragCrop(start, handle: .se, dx: 0.1, dy: 0.1, minWidth: 0.02, minHeight: 0.02)
        XCTAssertEqual(corner.x, 0.2, accuracy: 1e-12)
        XCTAssertEqual(corner.width, 0.6, accuracy: 1e-12)
        XCTAssertEqual(corner.height, 0.6, accuracy: 1e-12)
        let squeezed = AttachmentEdit.dragCrop(start, handle: .w, dx: 0.9, dy: 0, minWidth: 0.1, minHeight: 0.1)
        XCTAssertEqual(squeezed.x, 0.6, accuracy: 1e-12, "the left edge stops the minimum short of the right")
        XCTAssertEqual(squeezed.width, 0.1, accuracy: 1e-12)
        let top = AttachmentEdit.dragCrop(start, handle: .n, dx: 0.3, dy: -0.5, minWidth: 0.02, minHeight: 0.02)
        XCTAssertEqual(top.x, 0.2, accuracy: 1e-12, "an edge handle moves one side only")
        XCTAssertEqual(top.y, 0, accuracy: 1e-12)
        XCTAssertEqual(top.height, 0.7, accuracy: 1e-12)
    }

    func testColoursParseAndFormatAsWebHex() {
        XCTAssertEqual(AttachmentEdit.colors.first, "#E5484D", "red is the default")
        let red = AttachmentEdit.components("#E5484D")
        XCTAssertEqual(red?.red ?? 0, 229.0 / 255, accuracy: 1e-12)
        XCTAssertEqual(red?.green ?? 0, 72.0 / 255, accuracy: 1e-12)
        XCTAssertEqual(red?.blue ?? 0, 77.0 / 255, accuracy: 1e-12)
        XCTAssertNil(AttachmentEdit.components("#E5484"))
        XCTAssertNil(AttachmentEdit.components("red"))
        XCTAssertEqual(AttachmentEdit.hex(red: 229.0 / 255, green: 72.0 / 255, blue: 77.0 / 255), "#E5484D")
        XCTAssertEqual(AttachmentEdit.hex(red: 1.2, green: -0.1, blue: .nan), "#FF0000", "out-of-gamut picks clamp")
        for color in AttachmentEdit.colors { XCTAssertNotNil(AttachmentEdit.colorNames[color]) }
    }

    func testEditableStillTypes() {
        for type in ["image/jpeg", "image/png", "image/webp", "image/avif", "image/heic", "image/heif", "image/bmp", "image/tiff", "IMAGE/PNG; x=1"] {
            XCTAssertTrue(AttachmentEdit.editableImageType(type), type)
        }
        for type in ["image/gif", "image/svg+xml", "video/mp4", "audio/mpeg", "application/pdf", ""] {
            XCTAssertFalse(AttachmentEdit.editableImageType(type), type)
        }
    }

    func testEditedVideosEncodeTheirCropWithTheShortEdgeCapped() {
        let full = AttachmentEdit(rotation: 90).videoFrames(displayWidth: 3840, displayHeight: 2160, maxHeight: 1080)
        XCTAssertEqual(full.crop, PixelRect(left: 0, top: 0, width: 2160, height: 3840), "turned into portrait")
        XCTAssertTrue((full.width, full.height) == (1080, 1920))
        let half = AttachmentEdit(crop: EditRect(x: 0, y: 0, width: 0.5, height: 1)).videoFrames(displayWidth: 1920, displayHeight: 1080, maxHeight: 1440)
        XCTAssertTrue((half.width, half.height) == (960, 1080), "a crop under the cap keeps its pixels")
        let uncapped = AttachmentEdit(crop: EditRect(x: 0.1, y: 0.1, width: 0.5, height: 0.5)).videoFrames(displayWidth: 1001, displayHeight: 1001, maxHeight: 0)
        XCTAssertEqual(uncapped.crop.width % 2, 0)
        XCTAssertTrue((uncapped.width, uncapped.height) == (uncapped.crop.width, uncapped.crop.height))
    }

    func testStageLayoutFitsTheShownRegionAndMeasuresTheWholePicture() throws {
        let cropped = EditRect(x: 0.5, y: 0, width: 0.5, height: 1)
        let layout = try XCTUnwrap(AttachmentEditLayout.fit(pictureWidth: 4000, pictureHeight: 2000, region: cropped,
                                                            boxWidth: 532, boxHeight: 1032, padding: 16))
        XCTAssertEqual(layout.width, 500, accuracy: 1e-9)
        XCTAssertEqual(layout.height, 500, accuracy: 1e-9)
        XCTAssertEqual(layout.edge, 1000, accuracy: 1e-9, "sizes are fractions of the whole picture's longer edge")
        let point = layout.fraction(x: 250, y: 100, region: cropped)
        XCTAssertEqual(point.x, 0.75, accuracy: 1e-12)
        XCTAssertEqual(point.y, 0.2, accuracy: 1e-12)
        let back = layout.point(x: point.x, y: point.y, region: cropped)
        XCTAssertEqual(back.x, 250, accuracy: 1e-9)
        XCTAssertEqual(back.y, 100, accuracy: 1e-9)
        XCTAssertNil(AttachmentEditLayout.fit(pictureWidth: 0, pictureHeight: 10, region: .full, boxWidth: 10, boxHeight: 10, padding: 0))
    }

    func testTextFieldsCommitAddChangeAndRemoveMarks() {
        let field = AttachmentEditTextField(index: nil, x: 0.5, y: 0.5, value: "  Hello  ", color: "#FFFFFF", points: 32)
        let added = field.applied(to: .original, edge: 640)
        XCTAssertEqual(added.marks, [.text(TextMark(color: "#FFFFFF", size: 0.05, x: 0.5, y: 0.5, text: "Hello"))])
        XCTAssertEqual(AttachmentEditTextField(index: nil, x: 0, y: 0, value: " ", color: "#FFFFFF", points: 32).applied(to: added, edge: 640),
                       added, "an empty new field adds nothing")
        let changed = AttachmentEditTextField(index: 0, x: 0.5, y: 0.5, value: "Bye", color: "#000000", points: 64).applied(to: added, edge: 640)
        XCTAssertEqual(changed.marks, [.text(TextMark(color: "#000000", size: 0.1, x: 0.5, y: 0.5, text: "Bye"))])
        XCTAssertEqual(AttachmentEditTextField(index: 0, x: 0.5, y: 0.5, value: "", color: "#000000", points: 64).applied(to: changed, edge: 640),
                       .original, "emptying a text removes it")
        let long = AttachmentEditTextField(index: nil, x: 0, y: 0, value: String(repeating: "a", count: 250), color: "#FFFFFF", points: 32)
        guard case .text(let mark)? = long.applied(to: .original, edge: 640).marks.first else { return XCTFail("no text") }
        XCTAssertEqual(mark.text.count, AttachmentEdit.maxTextLength)
    }

    func testHistoryUndoesAndRedoesCommittedChanges() {
        var history = AttachmentEditHistory(.original)
        XCTAssertFalse(history.canUndo)
        history.commit(.original)
        XCTAssertFalse(history.canUndo, "no change is no step")
        let turned = AttachmentEdit.original.rotatedLeft()
        history.commit(turned)
        history.commit(turned.rotatedLeft())
        history.undo()
        XCTAssertEqual(history.current, turned)
        XCTAssertTrue(history.canRedo)
        history.commit(.original)
        XCTAssertFalse(history.canRedo, "a new change drops what was undone")
        XCTAssertEqual(history.current, .original)
        history.undo(); history.undo()
        XCTAssertEqual(history.current, .original)
        XCTAssertFalse(history.canUndo)
        history.redo()
        XCTAssertEqual(history.current, turned)
    }

    func testEditorOpensOnDrawUnlessThePreviewPicksATool() {
        XCTAssertEqual(AttachmentEditTool.allCases, [.crop, .draw, .text], "the switcher's and the preview's order")
        XCTAssertEqual(AttachmentEditTool.allCases.map(\.title), ["Crop", "Draw", "Text"])
        XCTAssertEqual(AttachmentEditTool.allCases.map(\.icon), ["crop", "brush", "type"])
        let file = URL(fileURLWithPath: "/tmp/photo.png")
        let pencil = AttachmentEditRequest(name: "photo.png", kind: .image, original: file, edit: .original) { _, _ in }
        XCTAssertEqual(pencil.tool, .draw)
        let cropping = AttachmentEditRequest(name: "photo.png", kind: .image, original: file, edit: .original, tool: .crop) { _, _ in }
        XCTAssertEqual(cropping.tool, .crop)
    }

    func testStagedTilesOfferSpoilerThenEditThenRemove() {
        XCTAssertEqual(StagedFileChip.actions(spoilerToggle: true, removable: true, editable: true), [.spoiler, .edit, .remove])
        XCTAssertEqual(StagedFileChip.actions(spoilerToggle: true, removable: true, editable: false), [.spoiler, .remove])
        XCTAssertEqual(StagedFileChip.actions(spoilerToggle: false, removable: true, editable: true), [.edit, .remove])
        XCTAssertEqual(StagedFileChip.actions(spoilerToggle: false, removable: true, editable: false), [.remove])
        XCTAssertEqual(StagedFileChip.actions(spoilerToggle: true, removable: false, editable: true), [.spoiler],
                       "while sending only the (locked) spoiler toggle stays")
        XCTAssertEqual(StagedFileChip.actions(spoilerToggle: true, removable: false, editable: false), [.spoiler])
        XCTAssertEqual(StagedFileChip.actions(spoilerToggle: false, removable: false, editable: true), [], "no bar while sending")
        XCTAssertEqual(StagedFileChip.actions(spoilerToggle: false, removable: false, editable: false), [])
    }

    func testBrushRingIsAsWideAsTheStrokeOnScreen() throws {
        // A 4000 × 2000 picture fitted at a quarter of its size: strokes are fractions of the 1000-point edge.
        let layout = try XCTUnwrap(AttachmentEditLayout.fit(pictureWidth: 4000, pictureHeight: 2000, region: .full,
                                                            boxWidth: 1032, boxHeight: 1032, padding: 16))
        XCTAssertEqual(layout.edge, 1000, accuracy: 1e-9)
        XCTAssertEqual(layout.strokeWidth(points: 12), 0.012, accuracy: 1e-12)
        for points in [AttachmentEdit.initialStrokeSize, 12, AttachmentEdit.strokeSizes.upperBound] {
            let painted = StrokeMark(color: "#E5484D", width: layout.strokeWidth(points: points), points: [0.5, 0.5]).width * layout.edge
            XCTAssertEqual(layout.brushDiameter(points: points), painted, accuracy: 1e-9, "\(points) points")
        }
        XCTAssertEqual(layout.brushDiameter(points: AttachmentEdit.strokeSizes.lowerBound), AttachmentEditLayout.minimumBrushRing,
                       "a thin brush still shows a ring")
        let cropped = try XCTUnwrap(AttachmentEditLayout.fit(pictureWidth: 4000, pictureHeight: 2000, region: EditRect(x: 0.5, y: 0, width: 0.5, height: 1),
                                                             boxWidth: 532, boxHeight: 1032, padding: 16))
        XCTAssertEqual(cropped.brushDiameter(points: 20), 20, accuracy: 1e-9, "a crop doesn't change the on-screen width")
    }

    func testBrushRingFollowsHoveringPointersOnly() {
        let start = Date(timeIntervalSince1970: 1000)
        var touch = AttachmentEditBrushPointer()
        touch.pressed(at: CGPoint(x: 10, y: 10), starting: true, now: start)
        touch.pressed(at: CGPoint(x: 20, y: 20), starting: false, now: start)
        XCTAssertNil(touch.location, "touches show no ring")
        touch.released()
        XCTAssertNil(touch.location)

        var mouse = AttachmentEditBrushPointer()
        mouse.hovered(at: CGPoint(x: 5, y: 5))
        XCTAssertEqual(mouse.location, CGPoint(x: 5, y: 5))
        mouse.pressed(at: CGPoint(x: 6, y: 6), starting: true, now: start)
        mouse.pressed(at: CGPoint(x: 40, y: 30), starting: false, now: start)
        XCTAssertEqual(mouse.location, CGPoint(x: 40, y: 30), "it sits at the stroke's tip")
        mouse.hoverEnded(now: start)
        mouse.pressed(at: CGPoint(x: 50, y: 30), starting: false, now: start)
        XCTAssertEqual(mouse.location, CGPoint(x: 50, y: 30), "and follows a drag off the picture")
        mouse.released()
        XCTAssertNil(mouse.location, "released outside the picture")

        var pad = AttachmentEditBrushPointer()
        pad.hovered(at: CGPoint(x: 5, y: 5))
        pad.hoverEnded(now: start)
        XCTAssertNil(pad.location, "gone when the pointer leaves")
        pad.pressed(at: CGPoint(x: 5, y: 5), starting: true, now: start.addingTimeInterval(0.1))
        XCTAssertEqual(pad.location, CGPoint(x: 5, y: 5), "an iPad click can end the hover just before the press")
        pad.released()
        pad.hovered(at: CGPoint(x: 8, y: 8))
        XCTAssertEqual(pad.location, CGPoint(x: 8, y: 8))
        pad.released()
        XCTAssertEqual(pad.location, CGPoint(x: 8, y: 8), "still hovering")

        var late = AttachmentEditBrushPointer()
        late.hovered(at: CGPoint(x: 5, y: 5))
        late.hoverEnded(now: start)
        late.pressed(at: CGPoint(x: 5, y: 5), starting: true, now: start.addingTimeInterval(1))
        XCTAssertNil(late.location, "a touch long after the pointer left")
    }
}
