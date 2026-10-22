package dev.pickforge.pickscribe

import android.Manifest
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.Settings
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    private val recorder = Recorder()

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
        MethodChannel(messenger, "pickscribe/system").setMethodCallHandler { call, result ->
            when (call.method) {
                "status" -> result.success(
                    mapOf("microphone" to micGranted(), "service" to serviceEnabled()),
                )
                "requestMicrophone" -> {
                    requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 1)
                    result.success(null)
                }
                "openAccessibilitySettings" -> {
                    startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        }
    }

    private fun micGranted() =
        checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    private fun serviceEnabled(): Boolean {
        val enabled = Settings.Secure.getString(
            contentResolver,
            Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES,
        ).orEmpty()
        val ours = ComponentName(this, DictationService::class.java).flattenToString()
        return enabled.split(':').any { it.equals(ours, ignoreCase = true) }
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
        super.onDestroy()
    }
}
