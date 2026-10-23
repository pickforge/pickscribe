package dev.pickforge.pickscribe

import android.Manifest
import android.accessibilityservice.AccessibilityService
import android.content.SharedPreferences
import android.content.res.Configuration
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
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
    private val main = Handler(Looper.getMainLooper())

    // Sends the audio to the pipeline while the user speaks, so it can transcribe finished parts.
    private val streamAudio = object : Runnable {
        override fun run() {
            if (state != State.LISTENING) return
            val pcm = recorder.takeNew()
            if (pcm.isNotEmpty()) pipeline.invokeMethod("audio", mapOf("session" to session, "pcm" to pcm))
            main.postDelayed(this, STREAM_MS)
        }
    }
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

    override fun onAccessibilityEvent(event: AccessibilityEvent) = refresh()

    override fun onInterrupt() = Unit

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (::bubble.isInitialized) bubble.applyPrefs()
    }

    override fun onDestroy() {
        // A result that arrives after this point must not touch the clipboard or the windows.
        if (state != State.IDLE) stopWork()
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
        // The pipeline loads the model now, while the user speaks.
        pipeline.invokeMethod("begin", mapOf("session" to session))
        main.postDelayed(streamAudio, STREAM_MS)
        bubble.view.listening(raw) { recorder.level }
    }

    private fun stop() {
        main.removeCallbacks(streamAudio)
        val rest = recorder.stop()
        val mine = session
        val problem = when {
            recorder.silenced -> R.string.microphone_unavailable
            recorder.length < Recorder.RATE / 4 -> R.string.too_short
            else -> null
        }
        if (problem != null) {
            pipeline.invokeMethod("cancel", null)
            say(problem)
            finish()
            return
        }
        state = State.THINKING
        bubble.view.thinking()
        val app = field?.packageName?.toString()
        pipeline.invokeMethod(
            "finish",
            mapOf("session" to session, "pcm" to rest, "raw" to raw, "app" to app),
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
        if (state == State.IDLE) return
        stopWork()
        session++
        say(R.string.cancelled)
        finish()
    }

    // Parts of a recording may be transcribing while it is still going, so both states cancel.
    private fun stopWork() {
        if (state == State.LISTENING) {
            main.removeCallbacks(streamAudio)
            recorder.stop()
        }
        pipeline.invokeMethod("cancel", null)
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
        private const val STREAM_MS = 200L
        private const val TAG = "PickScribe"
    }
}
