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

    // The Firebase identifiers below are resource values, which the Android
    // Gradle plugin leaves off unless asked.
    buildFeatures {
        resValues = true
    }

    defaultConfig {
        applicationId = "com.zelamar.app"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName

        // Push wake-ups (ADR-0017): the Firebase app's public identifiers,
        // given to release builds as environment variables. They become the
        // resources Firebase reads at start-up; without them there is no
        // push and nothing else changes.
        val firebase = mapOf(
            "google_app_id" to System.getenv("ZELAMAR_FIREBASE_APP_ID"),
            "google_api_key" to System.getenv("ZELAMAR_FIREBASE_API_KEY"),
            "project_id" to System.getenv("ZELAMAR_FIREBASE_PROJECT_ID"),
            "gcm_defaultSenderId" to System.getenv("ZELAMAR_FIREBASE_SENDER_ID"),
        )
        if (firebase.values.all { !it.isNullOrBlank() }) {
            firebase.forEach { (name, value) -> resValue("string", name, value!!) }
        }
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

dependencies {
    // Apache-2.0; it brings Google Play services (ADR-0017).
    implementation("com.google.firebase:firebase-messaging:24.1.0")
}
