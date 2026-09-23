// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "CaperApple",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .library(name: "CaperCore", targets: ["CaperCore"]),
        .executable(name: "CaperMacOS", targets: ["CaperMacOS"]),
        .executable(name: "CaperIOS", targets: ["CaperIOS"]),
    ],
    dependencies: [
        .package(url: "https://github.com/stasel/WebRTC.git", exact: "153.0.0"),
    ],
    targets: [
        .target(name: "CaperCore", dependencies: [.product(name: "WebRTC", package: "WebRTC")]),
        .executableTarget(name: "CaperMacOS", dependencies: ["CaperCore"]),
        .executableTarget(name: "CaperIOS", dependencies: ["CaperCore"]),
        .testTarget(name: "CaperCoreTests", dependencies: ["CaperCore"]),
    ]
)
