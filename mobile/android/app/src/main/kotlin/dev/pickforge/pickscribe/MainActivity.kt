package dev.pickforge.pickscribe

import android.Manifest
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import java.util.concurrent.Executors

class MainActivity : FlutterActivity() {
    private val recorder = Recorder()

    // App icons take a few milliseconds each, so they render off the main thread.
    private val iconWorker = Executors.newSingleThreadExecutor()

    override fun getCachedEngineId() = PickScribeApplication.ENGINE_ID

    // The dictation service keeps using the engine after the screen closes.
    override fun shouldDestroyEngineWithHost() = false

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        val messenger = flutterEngine.dartExecutor.binaryMessenger
        MethodChannel(messenger, "pickscribe/recorder").setMethodCallHandler { call, result ->
            when (call.method) {
                "start" -> startRecording(result)
                "stop" -> result.success(recorder.stop())
                else -> result.notImplemented()
            }
        }
        MethodChannel(messenger, "pickscribe/system").setMethodCallHandler(::system)
        MethodChannel(messenger, "pickscribe/bubble").setMethodCallHandler(::bubble)
    }

    private fun system(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "status" -> result.success(
                mapOf(
                    "microphone" to micGranted(),
                    "service" to serviceEnabled(),
                    "version" to packageManager.getPackageInfo(packageName, 0).versionName,
                ),
            )
            "requestMicrophone" -> {
                // After two refusals Android stops asking, so the app settings page is the way.
                if (shouldShowRequestPermissionRationale(Manifest.permission.RECORD_AUDIO) ||
                    !askedForMicrophone()
                ) {
                    rememberMicrophoneAsked()
                    requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 1)
                } else {
                    openAppSettings()
                }
                result.success(null)
            }
            "openAccessibilitySettings" -> {
                startActivity(accessibilitySettings())
                result.success(null)
            }
            "describeApps" -> {
                val packages = call.argument<List<String>>("packages").orEmpty()
                val iconPx = call.argument<Int>("iconPx") ?: 96
                iconWorker.execute {
                    val apps = Apps.describe(this, packages, iconPx)
                    runOnUiThread { result.success(apps) }
                }
            }
            "launchableApps" -> iconWorker.execute {
                val apps = Apps.launchable(this)
                runOnUiThread { result.success(apps) }
            }
            else -> result.notImplemented()
        }
    }

    private fun bubble(call: MethodCall, result: MethodChannel.Result) {
        val prefs = BubblePrefs(this)
        when (call.method) {
            "get" -> {}
            "setLook" -> prefs.setLook(
                call.argument<Int>("size"),
                call.argument<Double>("opacity")?.toFloat(),
            )
            "resume" -> prefs.resume()
            "block" -> call.argument<String>("package")?.let(prefs::block)
            "unblock" -> call.argument<String>("package")?.let(prefs::unblock)
            else -> {
                result.notImplemented()
                return
            }
        }
        result.success(
            mapOf(
                "size" to prefs.sizeDp,
                "opacity" to prefs.opacity.toDouble(),
                "blocked" to prefs.blockedApps.sorted(),
                "snoozedUntil" to prefs.snoozedUntil,
            ),
        )
    }

    private fun micGranted() =
        checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    private fun askedForMicrophone() =
        getSharedPreferences(STATE, MODE_PRIVATE).getBoolean(MIC_ASKED, false)

    private fun rememberMicrophoneAsked() =
        getSharedPreferences(STATE, MODE_PRIVATE).edit().putBoolean(MIC_ASKED, true).apply()

    private fun openAppSettings() = startActivity(
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", packageName, null)),
    )

    private fun serviceComponent() = ComponentName(this, DictationService::class.java).flattenToString()

    private fun serviceEnabled(): Boolean {
        val enabled = Settings.Secure.getString(
            contentResolver,
            Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES,
        ).orEmpty()
        return enabled.split(':').any { it.equals(serviceComponent(), ignoreCase = true) }
    }

    // Settings highlights the service in its list when it gets the component in these extras.
    private fun accessibilitySettings() = Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS).apply {
        val component = serviceComponent()
        putExtra(":settings:fragment_args_key", component)
        putExtra(":settings:show_fragment_args", Bundle().apply { putString(":settings:fragment_args_key", component) })
    }

    private fun startRecording(result: MethodChannel.Result) {
        if (!micGranted()) {
            requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 1)
            result.error("permission", "Allow the microphone, then try again.", null)
            return
        }
        try {
            recorder.start()
            result.success(null)
        } catch (error: IllegalStateException) {
            result.error("recorder", error.message, null)
        }
    }

    override fun onDestroy() {
        recorder.stop()
        iconWorker.shutdown()
        super.onDestroy()
    }

    companion object {
        private const val STATE = "app"
        private const val MIC_ASKED = "microphone_asked"
    }
}
