// Read-only companion; never linked into the published rendering fixture.
import CoreGraphics
import Foundation
import Metal

let devices: [[String: Any]] = MTLCopyAllDevices().map { device in
    [
        "name": device.name,
        "registryID": String(device.registryID),
        "architecture": device.architecture.name,
        "supportsMac2": device.supportsFamily(.mac2),
        "supportsMetal3": device.supportsFamily(.metal3),
        "lowPower": device.isLowPower,
        "headless": device.isHeadless,
        "unifiedMemory": device.hasUnifiedMemory,
        "recommendedMaxWorkingSetSize": device.recommendedMaxWorkingSetSize,
    ]
}
let data = try JSONSerialization.data(withJSONObject: [
    "defaultDevicePresent": MTLCreateSystemDefaultDevice() != nil,
    "devices": devices,
    // Preflight only: never request permission or launch a permission prompt.
    "screenCapturePreflight": CGPreflightScreenCaptureAccess(),
], options: [.prettyPrinted, .sortedKeys])
FileHandle.standardOutput.write(data)
FileHandle.standardOutput.write(Data([10]))
