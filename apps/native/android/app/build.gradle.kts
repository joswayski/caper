import java.net.URI

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

val firebaseEnabled = file("google-services.json").isFile
if (firebaseEnabled) apply(plugin = "com.google.gms.google-services")

val releaseStore = providers.environmentVariable("CAPER_ANDROID_KEYSTORE")
val releaseStorePassword = providers.environmentVariable("CAPER_ANDROID_KEYSTORE_PASSWORD")
val releaseKeyAlias = providers.environmentVariable("CAPER_ANDROID_KEY_ALIAS")
val releaseKeyPassword = providers.environmentVariable("CAPER_ANDROID_KEY_PASSWORD")
val releaseSigningInputs = listOf(releaseStore, releaseStorePassword, releaseKeyAlias, releaseKeyPassword)
val releaseSigningAvailable = releaseSigningInputs.all { it.isPresent }
val apiBaseUrl = providers.gradleProperty("caperApiBaseUrl").orElse("https://caper.chat")
val fixtureMode = providers.gradleProperty("caperFixtureMode").orElse("false")
// Native release run number (release.yml). Google Play and APK updates need a
// versionCode higher than the installed one; local builds stay at 1.
val buildNumber = providers.environmentVariable("CAPER_BUILD_NUMBER").map { it.toInt() }.orElse(1)

if (fixtureMode.get().toBoolean()) {
    val fixtureOrigin = URI(apiBaseUrl.get())
    require(
        fixtureOrigin.scheme == "http" && fixtureOrigin.host in setOf("localhost", "127.0.0.1", "::1") &&
            fixtureOrigin.rawUserInfo == null && fixtureOrigin.rawQuery == null && fixtureOrigin.rawFragment == null
    ) {
        "Fixture mode only permits an HTTP loopback API URL."
    }
}

android {
    namespace = "chat.caper.android"
    compileSdk = 36
    buildToolsVersion = "36.0.0"

    signingConfigs {
        if (releaseSigningAvailable) create("release") {
            storeFile = file(releaseStore.get())
            storePassword = releaseStorePassword.get()
            keyAlias = releaseKeyAlias.get()
            keyPassword = releaseKeyPassword.get()
        }
    }

    defaultConfig {
        applicationId = "chat.caper.android"
        minSdk = 26
        targetSdk = 36
        versionCode = buildNumber.get()
        versionName = if (buildNumber.get() > 1) "0.1.${buildNumber.get()}" else "0.1.0-dev"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // The Play bundle carries native symbols so Android vitals can show
        // readable stack traces for crashes in WebRTC or the audio pipeline.
        ndk { debugSymbolLevel = "FULL" }
        buildConfigField("boolean", "ENABLE_NATIVE_VOICE", "true")
        buildConfigField("boolean", "FIREBASE_ENABLED", firebaseEnabled.toString())
        externalNativeBuild { cmake { cppFlags += "-std=c++17" } }
    }

    buildTypes {
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
            buildConfigField("String", "API_BASE_URL", "\"${apiBaseUrl.get()}\"")
            buildConfigField("boolean", "FIXTURE_MODE", fixtureMode.get())
            manifestPlaceholders["usesCleartextTraffic"] = fixtureMode.get()
        }
        release {
            buildConfigField("String", "API_BASE_URL", "\"${apiBaseUrl.get()}\"")
            buildConfigField("boolean", "FIXTURE_MODE", "false")
            manifestPlaceholders["usesCleartextTraffic"] = "false"
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = if (releaseSigningAvailable) signingConfigs.getByName("release") else null
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }
    sourceSets["main"].apply {
        assets.srcDirs("../third_party", rootProject.file("../../../shared/emoji"), layout.buildDirectory.dir("generated/caper-fonts/assets"), layout.buildDirectory.dir("native-inputs/assets"))
        jniLibs.srcDir(layout.buildDirectory.dir("native-inputs/ort/jni"))
        res.srcDir(layout.buildDirectory.dir("generated/caper-fonts/res"))
    }
    ndkVersion = "27.2.12479018"
    externalNativeBuild { cmake { path = file("src/main/cpp/CMakeLists.txt"); version = "3.22.1" } }
    packaging.jniLibs.pickFirsts += "**/libonnxruntime.so"
    packaging.resources.excludes += setOf("/META-INF/{AL2.0,LGPL2.1}")
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions.jvmTarget = "17"
    lint.abortOnError = true
}

tasks.configureEach {
    if (name.startsWith("configureCMake") || name.startsWith("merge") && name.endsWith("Assets")) {
        doFirst { check(file("build/native-inputs/ort/headers/onnxruntime_cxx_api.h").isFile) { "Run prepare-audio.sh before Gradle." } }
    }
}

if (!releaseSigningAvailable) {
    tasks.configureEach {
        if (name in setOf("assembleRelease", "bundleRelease")) doFirst {
            error("All four release-signing variables are required; refusing to produce an unsigned release artifact.")
        }
    }
}

if (fixtureMode.get().toBoolean()) {
    tasks.configureEach {
        if (name in setOf("assembleRelease", "bundleRelease")) doFirst {
            error("Fixture mode is debug-only; refusing a release build.")
        }
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2025.09.01")
    implementation(composeBom)
    androidTestImplementation(composeBom)

    implementation("androidx.activity:activity-compose:1.11.0")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.4")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.4")
    implementation("androidx.core:core-ktx:1.17.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-play-services:1.10.2")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.9.0")
    implementation("com.squareup.okhttp3:okhttp:5.1.0")
    // Attachments: images keyed by attachment ID (so re-signed URLs reuse the
    // cache) over the app's OkHttp; Media3 plays video/audio. Originals upload
    // unchanged: the server's media worker does all compression.
    implementation("io.coil-kt.coil3:coil-compose:3.3.0")
    implementation("io.coil-kt.coil3:coil-network-okhttp:3.3.0")
    implementation("androidx.media3:media3-exoplayer:1.8.0")
    implementation("androidx.media3:media3-ui:1.8.0")
    // AOMedia libavif (dav1d decoder, ~0.9 MB per ABI): stored photos are mostly
    // AVIF, which Android only decodes itself from API 31. Used below that.
    implementation("org.aomedia.avif.android:avif:1.3.0.841110fd")
    // Word-level diffs for message edit history.
    implementation("io.github.java-diff-utils:java-diff-utils:4.16")
    implementation("io.github.webrtc-sdk:android:150.7871.01")
    implementation(platform("com.google.firebase:firebase-bom:34.3.0"))
    implementation("com.google.firebase:firebase-messaging")

    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
    testImplementation("com.squareup.okhttp3:mockwebserver:5.1.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.7.0")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-tooling")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
