plugins {
    id("com.android.application")
}

android {
    namespace = "dev.uc.s7"
    compileSdk = 37
    defaultConfig {
        applicationId = "dev.uc.s7"
        minSdk = 29
        targetSdk = 37
        versionCode = 1
        versionName = "0.1-spike"
        ndk { abiFilters += listOf("x86_64", "arm64-v8a") }
    }
}

dependencies {
    // UniFFI's generated Kotlin talks to the Rust library through JNA.
    implementation("net.java.dev.jna:jna:5.19.1@aar")
}
