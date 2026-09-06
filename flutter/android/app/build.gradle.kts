import javax.inject.Inject
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.FileSystemOperations
import org.gradle.api.tasks.InputDirectory
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.TaskAction

abstract class StageFlutterJni : DefaultTask() {
    @get:InputDirectory abstract val sourceDirectory: DirectoryProperty
    @get:OutputDirectory abstract val outputDirectory: DirectoryProperty
    @get:Inject abstract val fileOperations: FileSystemOperations
    @TaskAction fun stage() {
        fileOperations.sync { from(sourceDirectory); into(outputDirectory) }
    }
}

plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "so.shep.shep_mobile"
    compileSdk = 37
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "so.shep.shep_mobile"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    flavorDimensions += "workspace"
    productFlavors {
        create("production") { dimension = "workspace" }
        create("preview") {
            dimension = "workspace"
            applicationIdSuffix = ".preview"
            versionNameSuffix = "-preview"
        }
    }
    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            // Distribution signing must be supplied before store release.
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

// AGP 9 needs generated JNI inputs registered through its Variant API. Flutter
// 3.44's legacy source-set paths can be cached before the native asset exists.
// Stage its complete output (including AOT libapp) with an explicit task edge.
android.sourceSets.all {
    jniLibs.setSrcDirs(jniLibs.srcDirs.filterNot {
        it.path.replace('\\', '/').contains("/intermediates/flutter/")
    })
}
androidComponents.onVariants { variant ->
    val name = variant.name.replaceFirstChar { it.uppercaseChar() }
    val stage = tasks.register<StageFlutterJni>("stageShepJni$name") {
        dependsOn("copyJniLibsflutterBuild$name")
        sourceDirectory.set(layout.buildDirectory.dir("intermediates/flutter/${variant.name}/jniLibs"))
        outputDirectory.set(layout.buildDirectory.dir("generated/shepJni/${variant.name}"))
    }
    variant.sources.jniLibs?.addGeneratedSourceDirectory(stage, StageFlutterJni::outputDirectory)
}
