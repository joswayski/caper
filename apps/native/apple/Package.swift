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
        // Lossless WebP encoding for attachments (BSD; Resources/libwebp-LICENSE.txt).
        .package(url: "https://github.com/SDWebImage/libwebp-Xcode.git", exact: "1.6.0"),
    ],
    targets: [
        // AVIF photo encoding: libavif 1.4.2 with aom 3.15.1's encoder, one static
        // XCFramework built from pinned sources by build-libavif.sh (run prepare.sh
        // first; BSD, Resources/libavif-LICENSE.txt, libaom-LICENSE.txt, libaom-PATENTS.txt).
        .binaryTarget(name: "libavif", path: ".build/libavif-1.4.2/libavif.xcframework"),
        .target(name: "CaperCore", dependencies: [.product(name: "WebRTC", package: "WebRTC"), .product(name: "libwebp", package: "libwebp-Xcode"), "libavif"],
                resources: [.process("EmojiAssets"), .process("CaperAvatars.xcassets"), .process("InvitationAssets.xcassets"), .process("InvitationAssets")]),
        .executableTarget(name: "CaperMacOS", dependencies: ["CaperCore"]),
        .executableTarget(name: "CaperIOS", dependencies: ["CaperCore"]),
        .testTarget(name: "CaperCoreTests", dependencies: ["CaperCore", .product(name: "WebRTC", package: "WebRTC")]),
    ]
)
