// Observation only. No display configuration, application activation or capture.
import AppKit
import CoreGraphics
import Foundation

func rectangle(_ value: CGRect) -> [String: Double] {
    ["x": value.origin.x, "y": value.origin.y, "width": value.width, "height": value.height]
}

func modeRecord(_ mode: CGDisplayMode) -> [String: Any] {
    [
        "width": mode.width, "height": mode.height,
        "pixelWidth": mode.pixelWidth, "pixelHeight": mode.pixelHeight,
        "refreshRate": mode.refreshRate,
        "ioDisplayModeID": mode.ioDisplayModeID, "ioFlags": mode.ioFlags,
        "usableForDesktopGUI": mode.isUsableForDesktopGUI(),
    ]
}

var errors: [String] = []
let screens = NSScreen.screens
let mainScreen = NSScreen.main
let screenRecords: [[String: Any]] = screens.map { screen in
    let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber
    if number == nil { errors.append("NSScreenNumber missing") }
    return [
        "displayID": number.map { $0 as Any } ?? NSNull(),
        "isNSScreenMain": screen == mainScreen,
        "framePoints": rectangle(screen.frame),
        "visibleFramePoints": rectangle(screen.visibleFrame),
        "backingScaleFactor": screen.backingScaleFactor,
        "frameBackingPixels": rectangle(screen.convertRectToBacking(screen.frame)),
    ]
}
if screens.isEmpty { errors.append("NSScreen.screens empty") }
if mainScreen == nil { errors.append("NSScreen.main absent") }

var before: UInt32 = 0
let countStatus = CGGetOnlineDisplayList(0, nil, &before)
if countStatus != .success { errors.append("CGGetOnlineDisplayList count failed: \(countStatus.rawValue)") }
let limit: UInt32 = 64
var ids = [CGDirectDisplayID](repeating: 0, count: Int(limit))
var returned: UInt32 = 0
let listStatus = CGGetOnlineDisplayList(limit, &ids, &returned)
if listStatus != .success { errors.append("CGGetOnlineDisplayList failed: \(listStatus.rawValue)") }
if before > limit || returned >= limit { errors.append("Display enumeration reached 64-entry bound") }
let online = Array(ids.prefix(Int(min(returned, limit))))
// Include NSScreen IDs even if the CG online list is incomplete, for diagnosis.
let screenIDs = screens.compactMap {
    ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value
}
let displays: [[String: Any]] = Set(online + screenIDs).sorted().map { id in
    let current = CGDisplayCopyDisplayMode(id)
    let advertised = CGDisplayCopyAllDisplayModes(id, nil) as? [CGDisplayMode]
    if current == nil { errors.append("Current mode absent for display \(id)") }
    if advertised == nil { errors.append("Advertised modes absent for display \(id)") }
    if let advertised, advertised.count > 2048 { errors.append("Mode enumeration exceeded 2048 entries for \(id)") }
    return [
        "displayID": id, "isCGMain": id == CGMainDisplayID(),
        "online": CGDisplayIsOnline(id) != 0, "active": CGDisplayIsActive(id) != 0,
        "asleep": CGDisplayIsAsleep(id) != 0, "mirrorsDisplayID": CGDisplayMirrorsDisplay(id),
        "boundsGlobalDisplayCoordinates": rectangle(CGDisplayBounds(id)),
        "pixelsWide": CGDisplayPixelsWide(id), "pixelsHigh": CGDisplayPixelsHigh(id),
        "currentMode": current.map { modeRecord($0) as Any } ?? NSNull(),
        "advertisedModeCount": advertised.map { $0.count as Any } ?? NSNull(),
        "advertisedModes": advertised.map { $0.prefix(2048).map(modeRecord) as Any } ?? NSNull(),
    ]
}
var after: UInt32 = 0
let afterStatus = CGGetOnlineDisplayList(0, nil, &after)
if afterStatus != .success || before != returned || before != after {
    errors.append("Display counts changed or could not be rechecked")
}
let report: [String: Any] = [
    "observationOnly": true, "complete": errors.isEmpty, "errors": errors,
    "screens": screenRecords, "displays": displays,
    "onlineIDs": online, "onlineCountBefore": before, "onlineCountReturned": returned, "onlineCountAfter": after,
    "countStatus": countStatus.rawValue, "listStatus": listStatus.rawValue, "afterCountStatus": afterStatus.rawValue,
    "modeListOptions": "nil (API default advertised list)",
    "snapshotAtomic": false,
    "geometryPolicy": "Unchanged: NSScreen.main.visibleFrame must provide at least 1440x900 points. No mode is selected or tested here.",
]
do {
    let data = try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([10]))
    if !errors.isEmpty { exit(1) }
} catch {
    FileHandle.standardError.write(Data("Display inventory encoding failed: \(error)\n".utf8))
    exit(1)
}
