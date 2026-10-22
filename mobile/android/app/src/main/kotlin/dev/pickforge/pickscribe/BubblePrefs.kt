package dev.pickforge.pickscribe

import android.content.Context
import android.content.SharedPreferences
import java.util.Calendar

/** Bubble settings and snoozes, shared by the settings screen and the service. */
class BubblePrefs(context: Context) {
    private val prefs = context.getSharedPreferences(NAME, Context.MODE_PRIVATE)

    val sizeDp: Int get() = prefs.getInt(SIZE, 56)
    val opacity: Float get() = prefs.getFloat(OPACITY, 0.9f)
    val blockedApps: Set<String> get() = prefs.getStringSet(BLOCKED, emptySet()).orEmpty()
    private val snoozedUntil: Long get() = prefs.getLong(SNOOZED_UNTIL, 0)

    /** The saved position, or null before the first drag. */
    val position: Pair<Int, Int>?
        get() = if (prefs.contains(X)) prefs.getInt(X, 0) to prefs.getInt(Y, 0) else null

    fun savePosition(x: Int, y: Int) = prefs.edit().putInt(X, x).putInt(Y, y).apply()

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

    fun block(app: String) = prefs.edit().putStringSet(BLOCKED, blockedApps + app).apply()

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
    }
}
