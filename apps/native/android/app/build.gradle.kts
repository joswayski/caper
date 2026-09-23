plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

val releaseStore = providers.environmentVariable("CAPER_ANDROID_KEYSTORE")
val releaseStorePassword = providers.environmentVariable("CAPER_ANDROID_KEYSTORE_PASSWORD")
val releaseKeyAlias = providers.environmentVariable("CAPER_ANDROID_KEY_ALIAS")
val releaseKeyPassword = providers.environmentVariable("CAPER_ANDROID_KEY_PASSWORD")
val releaseSigningInputs = listOf(releaseStore, releaseStorePassword, releaseKeyAlias, releaseKeyPassword)
val releaseSigningAvailable = releaseSigningInputs.all { it.isPresent }

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
        versionCode = 1
        versionName = "0.1.0-dev"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        buildConfigField("String", "API_BASE_URL", "\"${providers.gradleProperty("caperApiBaseUrl").orElse("https://caper.chat").get()}\"")
        buildConfigField("boolean", "ENABLE_NATIVE_VOICE", providers.gradleProperty("caperEnableNativeVoice").orElse("false").get())
    }

    buildTypes {
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = if (releaseSigningAvailable) signingConfigs.getByName("release") else null
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }
    sourceSets["main"].assets.srcDir("../third_party")
    packaging.resources.excludes += setOf("/META-INF/{AL2.0,LGPL2.1}")
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions.jvmTarget = "17"
    lint.abortOnError = true
}

if (!releaseSigningAvailable) {
    tasks.configureEach {
        if (name in setOf("assembleRelease", "bundleRelease")) doFirst {
            error("All four release-signing variables are required; refusing to produce an unsigned release artifact.")
        }
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2025.09.01")
    implementation(composeBom)
    androidTestImplementation(composeBom)

    implementation("androidx.activity:activity-compose:1.11.0")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.4")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.4")
    implementation("androidx.core:core-ktx:1.17.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.9.0")
    implementation("com.squareup.okhttp3:okhttp:5.1.0")
    implementation("io.github.webrtc-sdk:android:150.7871.01")

    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
    testImplementation("com.squareup.okhttp3:mockwebserver:5.1.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-tooling")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
