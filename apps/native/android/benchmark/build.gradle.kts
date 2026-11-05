// Macrobenchmarks and the Baseline Profile generator for :app. They drive the app's
// `benchmarkRelease`/`nonMinifiedRelease` builds (release code, debug-signed, installed as
// chat.caper.android.benchmark) against the loopback bench fixture. See ../README.md.
plugins {
    id("com.android.test")
    id("androidx.baselineprofile")
}

android {
    namespace = "chat.caper.android.benchmark"
    compileSdk = 36
    buildToolsVersion = "36.0.0"

    defaultConfig {
        // Baseline Profile generation needs API 28+ (33+ without root).
        minSdk = 28
        targetSdk = 36
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    targetProjectPath = ":app"

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

// Only the release-code variants: root-level debug tasks (native CI's emulator `connectedDebugAndroidTest`)
// must not pick up macrobenchmarks.
androidComponents {
    beforeVariants { variant -> variant.enable = variant.buildType in setOf("benchmarkRelease", "nonMinifiedRelease") }
}

baselineProfile {
    // A real phone (or a runner's device) connected over adb; Gradle-managed emulators can't run in orbs.
    useConnectedDevices = true
}

dependencies {
    implementation("androidx.test.ext:junit:1.3.0")
    implementation("androidx.test.uiautomator:uiautomator:2.3.0")
    implementation("androidx.benchmark:benchmark-macro-junit4:1.5.0")
}
