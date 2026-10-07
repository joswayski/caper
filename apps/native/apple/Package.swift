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
        // AVIF photo encoding: libavif 1.0.0 with aom 3.0.0, built from source
        // (BSD; Resources/libavif-LICENSE.txt, libaom-LICENSE.txt, libvmaf-LICENSE.txt).
        // libavif-Xcode asks for libaom-Xcode "from: 3.0.0", which asks for
        // libvmaf-Xcode "from: 2.2.0"; both are pinned here too.
        .package(url: "https://github.com/SDWebImage/libavif-Xcode.git", exact: "1.0.0"),
        .package(url: "https://github.com/SDWebImage/libaom-Xcode.git", exact: "3.0.0"),
        .package(url: "https://github.com/SDWebImage/libvmaf-Xcode.git", exact: "2.3.1"),
    ],
    targets: [
        .target(name: "CaperCore", dependencies: [.product(name: "WebRTC", package: "WebRTC"), .product(name: "libwebp", package: "libwebp-Xcode"),
                                                  .product(name: "libavif", package: "libavif-Xcode"), .product(name: "libaom", package: "libaom-Xcode"),
                                                  .product(name: "libvmaf", package: "libvmaf-Xcode")],
                resources: [.process("EmojiAssets"), .process("CaperAvatars.xcassets"), .process("InvitationAssets.xcassets"), .process("InvitationAssets")]),
        .executableTarget(name: "CaperMacOS", dependencies: ["CaperCore"]),
        .executableTarget(name: "CaperIOS", dependencies: ["CaperCore"]),
        .testTarget(name: "CaperCoreTests", dependencies: ["CaperCore", .product(name: "WebRTC", package: "WebRTC")]),
    ]
)
