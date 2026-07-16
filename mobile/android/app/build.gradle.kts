plugins {
    id("com.android.application")
    id("kotlin-android")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

val uploadKeystorePath = providers.environmentVariable("BUZZ_ANDROID_UPLOAD_KEYSTORE_PATH").orNull
val uploadKeystorePassword = providers.environmentVariable("BUZZ_ANDROID_UPLOAD_KEYSTORE_PASSWORD").orNull
val uploadKeyAlias = providers.environmentVariable("BUZZ_ANDROID_UPLOAD_KEY_ALIAS").orNull
val uploadKeyPassword = providers.environmentVariable("BUZZ_ANDROID_UPLOAD_KEY_PASSWORD").orNull
val uploadSigningValues =
    mapOf(
        "BUZZ_ANDROID_UPLOAD_KEYSTORE_PATH" to uploadKeystorePath,
        "BUZZ_ANDROID_UPLOAD_KEYSTORE_PASSWORD" to uploadKeystorePassword,
        "BUZZ_ANDROID_UPLOAD_KEY_ALIAS" to uploadKeyAlias,
        "BUZZ_ANDROID_UPLOAD_KEY_PASSWORD" to uploadKeyPassword,
    )
val missingUploadSigningValues = uploadSigningValues.filterValues { it.isNullOrBlank() }.keys
val hasUploadSigning = missingUploadSigningValues.isEmpty()

// Release signing modes:
//   - "upload-keystore" (default): sign with the CI-vended upload keystore;
//     release builds fail loudly when any credential is missing.
//   - "external": deliberately produce an UNSIGNED release bundle for a
//     pipeline that signs through the central APK Signer service (Cashkite,
//     BOT-1234). No keystore material may be present in this mode.
val releaseSigningMode =
    providers.environmentVariable("BUZZ_ANDROID_RELEASE_SIGNING").orNull ?: "upload-keystore"
val externalReleaseSigning = releaseSigningMode == "external"
if (releaseSigningMode !in setOf("upload-keystore", "external")) {
    throw GradleException(
        "BUZZ_ANDROID_RELEASE_SIGNING must be \"upload-keystore\" or \"external\", got: " +
            releaseSigningMode,
    )
}
if (externalReleaseSigning && uploadSigningValues.values.any { !it.isNullOrBlank() }) {
    throw GradleException(
        "BUZZ_ANDROID_RELEASE_SIGNING=external must not be combined with " +
            "BUZZ_ANDROID_UPLOAD_* credentials; unset one of them.",
    )
}

android {
    namespace = "xyz.block.buzz.mobile"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = JavaVersion.VERSION_17.toString()
    }

    defaultConfig {
        applicationId = "xyz.block.buzz.mobile"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    signingConfigs {
        if (hasUploadSigning) {
            create("upload") {
                storeFile = file(requireNotNull(uploadKeystorePath))
                storePassword = uploadKeystorePassword
                keyAlias = uploadKeyAlias
                keyPassword = uploadKeyPassword
            }
        }
    }

    buildTypes {
        release {
            if (hasUploadSigning) {
                signingConfig = signingConfigs.getByName("upload")
            }
        }
    }
}

gradle.taskGraph.whenReady {
    val buildsRelease = allTasks.any { task ->
        task.project == project && task.name in setOf("assembleRelease", "bundleRelease")
    }
    if (buildsRelease && externalReleaseSigning) {
        // External signing: the unsigned bundle goes to the central APK
        // Signer. All keystore checks are intentionally skipped; the
        // guard above already rejected any BUZZ_ANDROID_UPLOAD_* values.
        return@whenReady
    }
    if (buildsRelease && !hasUploadSigning) {
        throw GradleException(
            "Release builds require Android upload signing credentials. Missing: " +
                missingUploadSigningValues.sorted().joinToString(", ") +
                ". For central APK Signer pipelines set BUZZ_ANDROID_RELEASE_SIGNING=external.",
        )
    }
    if (buildsRelease) {
        val configuredKeystore = File(requireNotNull(uploadKeystorePath))
        if (!configuredKeystore.isAbsolute) {
            throw GradleException(
                "BUZZ_ANDROID_UPLOAD_KEYSTORE_PATH must be absolute: $configuredKeystore",
            )
        }
        val keystore = file(configuredKeystore)
        val repositoryRoot = rootProject.projectDir.parentFile.parentFile.canonicalFile
        if (keystore.canonicalFile.toPath().startsWith(repositoryRoot.toPath())) {
            throw GradleException(
                "BUZZ_ANDROID_UPLOAD_KEYSTORE_PATH must be outside the repository: $keystore",
            )
        }
        if (!keystore.isFile || !keystore.canRead()) {
            throw GradleException(
                "BUZZ_ANDROID_UPLOAD_KEYSTORE_PATH is not a readable file: $keystore",
            )
        }
    }
}

flutter {
    source = "../.."
}
