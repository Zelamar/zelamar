plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "com.zelamar.app"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        applicationId = "com.zelamar.app"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    // Release builds are signed with Zelamar's upload key when CI provides
    // it (ZELAMAR_KEYSTORE, ZELAMAR_KEYSTORE_PASSWORD; see release.yml), so
    // every beta installs over the previous one. Local builds fall back to
    // the debug key.
    val keystore = System.getenv("ZELAMAR_KEYSTORE")
    val keystorePassword = System.getenv("ZELAMAR_KEYSTORE_PASSWORD")
    signingConfigs {
        if (keystore != null && keystorePassword != null) {
            create("zelamar") {
                storeFile = file(keystore)
                storePassword = keystorePassword
                keyAlias = "zelamar"
                keyPassword = keystorePassword
                storeType = "pkcs12"
            }
        }
    }

    buildTypes {
        release {
            signingConfig =
                signingConfigs.findByName("zelamar") ?: signingConfigs.getByName("debug")
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}
