plugins {
    id("com.android.library")
}

android {
    namespace = "uz.torex.torex_local_store"
    compileSdk = 35

    defaultConfig {
        minSdk = 21

        // Support all ABIs
        ndk {
            abiFilters += listOf("armeabi-v7a", "arm64-v8a", "x86_64")
        }

        consumerProguardFiles("proguard-rules.pro")
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
            // Pre-built .so files from jniLibs directory
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }
}


