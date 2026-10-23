package dev.pickforge.pickscribe

import android.Manifest
import android.accessibilityservice.AccessibilityService
import android.content.SharedPreferences
import android.content.res.Configuration
import android.content.pm.PackageManager
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
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

    // The app the bubble was last shown over, for "Always in this app" after focus moves.
    private var bubbleApp: String? = null

    // Each dictation gets a number, so a result that arrives after a cancel is dropped.
    private var session = 0
    private var lastVisibility: String? = null
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

    override fun onAccessibilityEvent(event: AccessibilityEvent) {
        if (event.packageName?.toString() != packageName && mayCopy(event)) TextInserter.keepClip()
        refresh()
    }

    // A copy comes with a tap, a text selection, or a window change. Our own paste only moves
    // the cursor.
    private fun mayCopy(event: AccessibilityEvent) = when (event.eventType) {
        AccessibilityEvent.TYPE_VIEW_CLICKED, AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED -> true
        AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED -> event.fromIndex != event.toIndex
        else -> false
    }

    override fun onInterrupt() = Unit

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (::bubble.isInitialized) bubble.applyPrefs()
    }

    override fun onDestroy() {
        // A result that arrives after this point must not touch the clipboard or the windows.
        when (state) {
            State.LISTENING -> recorder.stop()
            State.THINKING -> pipeline.invokeMethod("cancel", null)
            State.IDLE -> Unit
        }
        session++
        state = State.IDLE
        field = null
        // The system can unbind the service before it connects.
        if (::prefs.isInitialized) prefs.stopListening(prefsListener)
        if (::bubble.isInitialized) bubble.destroy()
        super.onDestroy()
    }

    /** Shows the bubble over a focused, editable, non-password field while the keyboard is up. */
    private fun refresh() {
        if (state != State.IDLE) return
        val focused = findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
        val app = focused?.packageName?.toString()
        val hiddenBecause = when {
            focused == null -> "no field has input focus"
            !focused.isEditable -> "the focused view is not editable"
            focused.isPassword -> "the field is a password"
            !keyboardVisible() -> "the keyboard is closed"
            !prefs.allows(app) -> "the bubble is snoozed or blocked here"
            else -> null
        }
        logVisibility(app, hiddenBecause)
        if (hiddenBecause == null) {
            bubbleApp = app
            bubble.show()
        } else {
            bubble.hide()
        }
    }

    // Logs only the app and the reason, never field contents, and only when they change.
    private fun logVisibility(app: String?, hiddenBecause: String?) {
        val line = if (hiddenBecause == null) "bubble shown in $app" else "bubble hidden in $app: $hiddenBecause"
        if (line == lastVisibility) return
        lastVisibility = line
        Log.d(TAG, line)
    }

    private fun keyboardVisible() =
        windows.any { it.type == AccessibilityWindowInfo.TYPE_INPUT_METHOD }

    override fun onTap() {
        when (state) {
            State.IDLE -> start(raw = false)
            State.LISTENING -> stop()
            State.THINKING -> cancel()
        }
    }

    override fun onLongPress() {
        if (state == State.IDLE) start(raw = true)
    }

    override fun onDropped() {
        if (state == State.IDLE) bubble.showSnoozeMenu() else cancel()
    }

    override fun onSnooze(choice: Bubble.Snooze) {
        when (choice) {
            Bubble.Snooze.MINUTES_15 -> prefs.snooze(15)
            Bubble.Snooze.HOUR -> prefs.snooze(60)
            Bubble.Snooze.TOMORROW -> prefs.snoozeUntilTomorrow()
            Bubble.Snooze.ALWAYS_HERE -> bubbleApp?.let(prefs::block)
        }
        refresh()
    }

    private fun start(raw: Boolean) {
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            say(R.string.microphone_needed)
            return
        }
        field = findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
        try {
            recorder.start()
        } catch (error: IllegalStateException) {
            say(R.string.microphone_unavailable)
            return
        }
        this.raw = raw
        session++
        state = State.LISTENING
        bubble.view.listening(raw) { recorder.level }
    }

    private fun stop() {
        val pcm = recorder.stop()
        val mine = session
        if (recorder.silenced) {
            say(R.string.microphone_unavailable)
            finish()
            return
        }
        if (pcm.size < Recorder.RATE / 4) {
            say(R.string.too_short)
            finish()
            return
        }
        state = State.THINKING
        bubble.view.thinking()
        val app = field?.packageName?.toString()
        pipeline.invokeMethod(
            "process",
            mapOf("pcm" to pcm, "raw" to raw, "app" to app),
            object : MethodChannel.Result {
                override fun success(result: Any?) {
                    if (mine == session) deliver(result as? Map<*, *>)
                }

                override fun error(code: String, message: String?, details: Any?) {
                    if (mine != session) return
                    say(message ?: code)
                    finish()
                }

                override fun notImplemented() {
                    if (mine == session) finish()
                }
            },
        )
    }

    /** Stops a recording or drops the transcription in progress, without inserting anything. */
    private fun cancel() {
        when (state) {
            State.IDLE -> return
            State.LISTENING -> recorder.stop()
            State.THINKING -> pipeline.invokeMethod("cancel", null)
        }
        session++
        say(R.string.cancelled)
        finish()
    }

    private fun deliver(result: Map<*, *>?) {
        val text = result?.get("text") as? String
        val id = result?.get("id") as? String
        val saved = result?.get("saved") != false
        if (text.isNullOrBlank()) {
            say(R.string.nothing_heard)
        } else {
            val inserted = TextInserter.insert(this, field, text)
            if (!inserted) say(if (saved) R.string.paste_failed else R.string.paste_failed_unsaved)
            if (id != null) pipeline.invokeMethod("delivered", mapOf("id" to id, "ok" to inserted))
        }
        finish()
    }

    private fun finish() {
        field = null
        state = State.IDLE
        bubble.view.idle()
        refresh()
    }

    private fun say(message: Int) = bubble.say(getString(message))

    private fun say(message: String) = bubble.say(message)

    companion object {
        const val PIPELINE_CHANNEL = "pickscribe/pipeline"
        private const val TAG = "PickScribe"
    }
}
