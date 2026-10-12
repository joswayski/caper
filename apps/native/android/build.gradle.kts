buildscript {
    dependencies {
        // Align AGP's built-in Kotlin compiler with the Compose and serialization plugins.
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.2.20")
    }
}

plugins {
    id("com.android.application") version "9.0.1" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.2.20" apply false
    id("org.jetbrains.kotlin.plugin.serialization") version "2.2.20" apply false
    id("com.google.gms.google-services") version "4.4.3" apply false
}
