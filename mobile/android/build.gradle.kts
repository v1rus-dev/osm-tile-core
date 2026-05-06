plugins {
    id("com.android.library")
    kotlin("android")
}

android {
    namespace = "com.osmtilecore"
    compileSdk = 35

    defaultConfig {
        minSdk = 23
    }

    sourceSets {
        getByName("main") {
            java.srcDir("src/main/java")
            jniLibs.srcDir("src/main/jniLibs")
        }
    }
}

dependencies {
    implementation("androidx.annotation:annotation:1.9.1")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
}
