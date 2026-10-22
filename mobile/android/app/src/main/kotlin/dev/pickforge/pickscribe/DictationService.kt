package dev.pickforge.pickscribe

import android.Manifest
import android.accessibilityservice.AccessibilityService
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import android.widget.Toast
import io.flutter.embedding.engine.FlutterEngineCache
import io.flutter.plugin.common.MethodChannel

/**
 * Shows the bubble while a text field has the keyboard, records when the bubble is tapped,
 * hands the audio to the Dart pipeline, and puts the result in the field.
 */
class DictationService : AccessibilityService(), Bubble.Actions {
    private enum class State { IDLE, LISTENING, THINKING }

    private lateinit var prefs: BubblePrefs
    private lateinit var bubble: Bubble
    private lateinit var pipeline: MethodChannel
    private val recorder = Recorder()
    private var state = State.IDLE
    private var raw = false
    private var field: AccessibilityNodeInfo? = null
    private val prefsListener = SharedPreferences.OnSharedPreferenceChangeListener { _, _ ->
        bubble.applyPrefs()
        refresh()
    }

    override fun onServiceConnected() {
        prefs = BubblePrefs(this)
        bubble = Bubble(this, prefs, this)
        prefs.listen(prefsListener)
        val engine = FlutterEngineCache.getInstance().get(PickScribeApplication.ENGINE_ID)
            ?: error("the Flutter engine starts with the application")
        pipeline = MethodChannel(engine.dartExecutor.binaryMessenger, PIPELINE_CHANNEL)
        refresh()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent) = refresh()

    override fun onInterrupt() = Unit

    override fun onDestroy() {
        if (state == State.LISTENING) recorder.stop()
        prefs.stopListening(prefsListener)
        bubble.destroy()
        super.onDestroy()
    }

    /** Shows the bubble over a focused, editable, non-password field while the keyboard is up. */
    private fun refresh() {
        if (state != State.IDLE) return
        val focused = findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
        val visible = focused != null &&
            focused.isEditable &&
            !focused.isPassword &&
            keyboardVisible() &&
            prefs.allows(focused.packageName?.toString())
        if (visible) bubble.show() else bubble.hide()
    }

    private fun keyboardVisible() =
        windows.any { it.type == AccessibilityWindowInfo.TYPE_INPUT_METHOD }

    override fun onTap() {
        when (state) {
            State.IDLE -> start(raw = false)
            State.LISTENING -> stop()
            State.THINKING -> Unit
        }
    }

    override fun onLongPress() {
        if (state == State.IDLE) start(raw = true)
    }

    override fun onSnooze(choice: Bubble.Snooze) {
        when (choice) {
            Bubble.Snooze.MINUTES_15 -> prefs.snooze(15)
            Bubble.Snooze.HOUR -> prefs.snooze(60)
            Bubble.Snooze.TOMORROW -> prefs.snoozeUntilTomorrow()
            Bubble.Snooze.ALWAYS_HERE ->
                findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.packageName?.toString()?.let(prefs::block)
        }
        refresh()
    }

    private fun start(raw: Boolean) {
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            toast(R.string.microphone_needed)
            return
        }
        field = findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
        try {
            recorder.start()
        } catch (error: IllegalStateException) {
            toast(R.string.microphone_unavailable)
            return
        }
        this.raw = raw
        state = State.LISTENING
        bubble.view.listening(raw) { recorder.level }
    }

    private fun stop() {
        val pcm = recorder.stop()
        state = State.THINKING
        bubble.view.thinking()
        val app = field?.packageName?.toString()
        pipeline.invokeMethod(
            "process",
            mapOf("pcm" to pcm, "raw" to raw, "app" to app),
            object : MethodChannel.Result {
                override fun success(result: Any?) = deliver(result as? String)

                override fun error(code: String, message: String?, details: Any?) {
                    toastText(message ?: code)
                    finish()
                }

                override fun notImplemented() = finish()
            },
        )
    }

    private fun deliver(text: String?) {
        val target = field
        if (!text.isNullOrEmpty() && target != null && !TextInserter.insert(this, target, text)) {
            toast(R.string.paste_failed)
        }
        finish()
    }

    private fun finish() {
        field = null
        state = State.IDLE
        bubble.view.idle()
        refresh()
    }

    private fun toast(message: Int) = Toast.makeText(this, message, Toast.LENGTH_SHORT).show()

    private fun toastText(message: String) = Toast.makeText(this, message, Toast.LENGTH_SHORT).show()

    companion object {
        const val PIPELINE_CHANNEL = "pickscribe/pipeline"
    }
}
