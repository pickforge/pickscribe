package dev.pickforge.pickscribe

import android.content.Context
import android.content.SharedPreferences
import java.util.Calendar

/** Bubble settings and snoozes, shared by the settings screen and the service. */
class BubblePrefs(context: Context) {
    private val prefs = context.getSharedPreferences(NAME, Context.MODE_PRIVATE)

    val sizeDp: Int get() = prefs.getInt(SIZE, DEFAULT_SIZE_DP)
    val opacity: Float get() = prefs.getFloat(OPACITY, DEFAULT_OPACITY)
    val blockedApps: Set<String> get() = prefs.getStringSet(BLOCKED, emptySet()).orEmpty()
    val snoozedUntil: Long get() = prefs.getLong(SNOOZED_UNTIL, 0)

    /** The saved position, or null before the first drag. */
    val position: Pair<Int, Int>?
        get() = if (prefs.contains(X)) prefs.getInt(X, 0) to prefs.getInt(Y, 0) else null

    fun savePosition(x: Int, y: Int) = prefs.edit().putInt(X, x).putInt(Y, y).apply()

    /** Sets the look, keeping each value inside the range the settings screen offers. */
    fun setLook(sizeDp: Int?, opacity: Float?) {
        val edit = prefs.edit()
        sizeDp?.let { edit.putInt(SIZE, it.coerceIn(MIN_SIZE_DP, MAX_SIZE_DP)) }
        opacity?.let { edit.putFloat(OPACITY, it.coerceIn(MIN_OPACITY, 1f)) }
        edit.apply()
    }

    fun allows(app: String?, now: Long = System.currentTimeMillis()): Boolean =
        now >= snoozedUntil && (app == null || app !in blockedApps)

    fun snooze(minutes: Int, now: Long = System.currentTimeMillis()) =
        prefs.edit().putLong(SNOOZED_UNTIL, now + minutes * 60_000L).apply()

    /** Snoozes until 6:00 the next morning, local time. */
    fun snoozeUntilTomorrow(now: Calendar = Calendar.getInstance()) {
        val morning = (now.clone() as Calendar).apply {
            add(Calendar.DAY_OF_YEAR, 1)
            set(Calendar.HOUR_OF_DAY, 6)
            set(Calendar.MINUTE, 0)
            set(Calendar.SECOND, 0)
            set(Calendar.MILLISECOND, 0)
        }
        prefs.edit().putLong(SNOOZED_UNTIL, morning.timeInMillis).apply()
    }

    fun resume() = prefs.edit().remove(SNOOZED_UNTIL).apply()

    fun block(app: String) = prefs.edit().putStringSet(BLOCKED, blockedApps + app).apply()

    fun unblock(app: String) = prefs.edit().putStringSet(BLOCKED, blockedApps - app).apply()

    fun listen(listener: SharedPreferences.OnSharedPreferenceChangeListener) =
        prefs.registerOnSharedPreferenceChangeListener(listener)

    fun stopListening(listener: SharedPreferences.OnSharedPreferenceChangeListener) =
        prefs.unregisterOnSharedPreferenceChangeListener(listener)

    companion object {
        const val NAME = "bubble"
        const val SIZE = "size_dp"
        const val OPACITY = "opacity"
        const val BLOCKED = "blocked_apps"
        const val SNOOZED_UNTIL = "snoozed_until"
        private const val X = "x"
        private const val Y = "y"
        const val DEFAULT_SIZE_DP = 56
        const val MIN_SIZE_DP = 40
        const val MAX_SIZE_DP = 80
        const val DEFAULT_OPACITY = 0.9f
        const val MIN_OPACITY = 0.3f
    }
}
