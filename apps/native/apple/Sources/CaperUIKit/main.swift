import UIKit

// An explicit entry point, so launch timing separates loading (before main)
// from Caper's own startup work.
CaperUIKitLaunch.mark("main")
UIApplicationMain(CommandLine.argc, CommandLine.unsafeArgv, nil, NSStringFromClass(CaperUIKitAppDelegate.self))
