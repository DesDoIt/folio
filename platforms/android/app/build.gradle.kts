import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// The reading engine is the shared Rust crate (../engine), reached through ../bindings.
// the repository root: platforms/android -> ../..
val repoRoot = rootProject.projectDir.parentFile.parentFile
val engineOut = layout.buildDirectory.dir("engine")
val buildEngine by tasks.registering(Exec::class) {
    description = "Builds the Rust engine for Android and generates its Kotlin API"
    workingDir = repoRoot
    commandLine("sh", "platforms/android/build-engine.sh")
    environment("ANDROID_HOME", android.sdkDirectory.absolutePath)
    inputs.dir(File(repoRoot, "engine/src"))
    inputs.dir(File(repoRoot, "bindings/src"))
    inputs.files(File(repoRoot, "Cargo.lock"), File(repoRoot, "engine/Cargo.toml"), File(repoRoot, "bindings/Cargo.toml"))
    outputs.dir(engineOut)
}
tasks.named("preBuild") { dependsOn(buildEngine) }

val keystoreProps = Properties().apply {
    val f = rootProject.file("keystore.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}

android {
    namespace = "io.folio.reader"
    compileSdk = 35

    defaultConfig {
        applicationId = "io.folio.reader"
        minSdk = 24
        targetSdk = 35
        // release builds get these from the tag (see .github/workflows/release.yml)
        versionCode = (findProperty("folioVersionCode") as String?)?.toInt() ?: 1
        versionName = (findProperty("folioVersionName") as String?) ?: "0.1.0"
    }

    signingConfigs {
        if (keystoreProps.isNotEmpty()) {
            create("release") {
                storeFile = rootProject.file(keystoreProps.getProperty("storeFile"))
                storePassword = keystoreProps.getProperty("storePassword")
                keyAlias = keystoreProps.getProperty("keyAlias")
                keyPassword = keystoreProps.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release")
        }
    }

    sourceSets["main"].apply {
        jniLibs.srcDir(engineOut.map { it.dir("jniLibs") })
        java.srcDir(engineOut.map { it.dir("kotlin") })
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
    }
    packaging {
        jniLibs.useLegacyPackaging = false
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.12.01")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    implementation("androidx.work:work-runtime-ktx:2.10.0")
    // UniFFI's generated Kotlin calls the engine through JNA
    implementation("net.java.dev.jna:jna:5.15.0@aar")
    debugImplementation("androidx.compose.ui:ui-tooling")
}
