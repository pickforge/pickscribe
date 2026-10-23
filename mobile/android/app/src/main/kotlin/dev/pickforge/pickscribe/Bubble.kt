package dev.pickforge.pickscribe

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.content.res.ColorStateList
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import kotlin.math.abs
import kotlin.math.hypot

/**
 * Owns the bubble window and its gestures. A tap goes to the service, a long press starts a raw
 * dictation, and dropping the bubble on the X at the bottom goes to the service too, which
 * cancels a dictation or opens the snooze menu.
 */
class Bubble(
    private val context: Context,
    private val prefs: BubblePrefs,
    private val actions: Actions,
) {
    interface Actions {
        fun onTap()
        fun onLongPress()
        fun onDropped()
        fun onSnooze(choice: Snooze)
    }

    enum class Snooze { MINUTES_15, HOUR, TOMORROW, ALWAYS_HERE }

    val view = BubbleView(context)
    private val windows = context.getSystemService(WindowManager::class.java)
    private val density = context.resources.displayMetrics.density
    private val params = overlayParams(sizePx(), sizePx()).apply { gravity = Gravity.TOP or Gravity.START }
    private var shown = false
    private var target: TextView? = null
    private var longPressCheck: Runnable? = null
    private var menu: View? = null
    private var hint: TextView? = null
    private val hideHint = Runnable { removeHint() }

    init {
        attachGestures()
    }

    private fun sizePx() = (prefs.sizeDp * density).toInt()

    private fun dp(value: Int) = (value * density).toInt()

    fun show() {
        if (shown || menu != null) return
        applyPrefs()
        windows.addView(view, params)
        shown = true
    }

    fun hide() {
        if (!shown) return
        // A removed view gets no ACTION_UP, so a pending long press must not outlive it.
        longPressCheck?.let(view::removeCallbacks)
        hideTarget()
        windows.removeView(view)
        shown = false
    }

    /** Applies size and opacity changes from the settings screen. */
    fun applyPrefs() {
        params.width = sizePx()
        params.height = sizePx()
        view.alpha = prefs.opacity
        placeAtSaved()
    }

    private fun placeAtSaved() {
        val saved = prefs.position
        val screen = context.resources.displayMetrics
        place(
            saved?.first ?: (screen.widthPixels - params.width - dp(8)),
            saved?.second ?: (screen.heightPixels * 0.42f).toInt(),
        )
        if (shown) windows.updateViewLayout(view, params)
    }

    // Keeps the whole bubble on screen, since a position saved in another orientation or on
    // another display can lie outside this one.
    private fun place(x: Int, y: Int) {
        val screen = context.resources.displayMetrics
        params.x = x.coerceIn(0, maxOf(0, screen.widthPixels - params.width))
        params.y = y.coerceIn(0, maxOf(0, screen.heightPixels - params.height))
    }

    /** Shows a short message beside the bubble, since toasts from a background app are hidden. */
    fun say(message: String) {
        removeHint()
        val label = TextView(context).apply {
            text = message
            textSize = 14f
            setTextColor(TEXT)
            setPadding(dp(14), dp(9), dp(14), dp(9))
            maxWidth = dp(260)
            background = GradientDrawable().apply {
                cornerRadius = dp(18).toFloat()
                setColor(PANEL)
                setStroke(dp(1), BORDER)
            }
        }
        label.measure(
            View.MeasureSpec.makeMeasureSpec(dp(260), View.MeasureSpec.AT_MOST),
            View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED),
        )
        val screen = context.resources.displayMetrics
        val gap = dp(10)
        val onRight = params.x + params.width / 2 > screen.widthPixels / 2
        val layout = overlayParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            x = if (onRight) params.x - label.measuredWidth - gap else params.x + params.width + gap
            x = x.coerceIn(gap, screen.widthPixels - label.measuredWidth - gap)
            y = params.y + (params.height - label.measuredHeight) / 2
            flags = flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
        }
        windows.addView(label, layout)
        hint = label
        label.postDelayed(hideHint, HINT_MS)
    }

    private fun removeHint() {
        hint?.let {
            it.removeCallbacks(hideHint)
            windows.removeView(it)
        }
        hint = null
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun attachGestures() {
        val slop = ViewConfiguration.get(context).scaledTouchSlop
        val longPress = ViewConfiguration.getLongPressTimeout().toLong()
        var downX = 0f
        var downY = 0f
        var startX = 0
        var startY = 0
        var dragging = false
        var pressedLong = false
        val longPressCheck = Runnable {
            if (!shown) return@Runnable
            pressedLong = true
            view.performHapticFeedback(HapticFeedbackConstants.LONG_PRESS)
            actions.onLongPress()
        }
        this.longPressCheck = longPressCheck
        view.setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    downX = event.rawX
                    downY = event.rawY
                    startX = params.x
                    startY = params.y
                    dragging = false
                    pressedLong = false
                    view.postDelayed(longPressCheck, longPress)
                }
                MotionEvent.ACTION_MOVE -> {
                    val dx = event.rawX - downX
                    val dy = event.rawY - downY
                    if (!dragging && !pressedLong && (abs(dx) > slop || abs(dy) > slop)) {
                        dragging = true
                        view.removeCallbacks(longPressCheck)
                        removeHint()
                        showTarget()
                    }
                    if (dragging && shown) {
                        params.x = startX + dx.toInt()
                        params.y = startY + dy.toInt()
                        windows.updateViewLayout(view, params)
                        highlightTarget(isOverTarget())
                    }
                }
                MotionEvent.ACTION_UP -> {
                    view.removeCallbacks(longPressCheck)
                    when {
                        dragging -> finishDrag()
                        !pressedLong -> {
                            view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY)
                            actions.onTap()
                        }
                    }
                }
                MotionEvent.ACTION_CANCEL -> {
                    view.removeCallbacks(longPressCheck)
                    if (dragging) finishDrag()
                }
            }
            true
        }
    }

    private fun finishDrag() {
        val overTarget = isOverTarget()
        hideTarget()
        if (overTarget) {
            placeAtSaved()
            actions.onDropped()
        } else {
            place(params.x, params.y)
            if (shown) windows.updateViewLayout(view, params)
            prefs.savePosition(params.x, params.y)
        }
    }

    private fun targetCenter(): Pair<Float, Float> {
        val screen = context.resources.displayMetrics
        return screen.widthPixels / 2f to screen.heightPixels - TARGET_MARGIN_DP * density
    }

    private fun isOverTarget(): Boolean {
        val (tx, ty) = targetCenter()
        val cx = params.x + params.width / 2f
        val cy = params.y + params.height / 2f
        return hypot(cx - tx, cy - ty) < TARGET_DP * density
    }

    private fun showTarget() {
        val size = dp(TARGET_DP)
        val x = TextView(context).apply {
            text = "✕"
            textSize = 20f
            gravity = Gravity.CENTER
            setTextColor(TEXT)
        }
        val (tx, ty) = targetCenter()
        val layout = overlayParams(size, size).apply {
            gravity = Gravity.TOP or Gravity.START
            this.x = (tx - size / 2f).toInt()
            this.y = (ty - size / 2f).toInt()
            flags = flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
        }
        windows.addView(x, layout)
        target = x
        highlightTarget(false)
    }

    private fun highlightTarget(over: Boolean) {
        target?.background = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(if (over) EMBER else PANEL)
            setStroke(dp(1), if (over) EMBER else BORDER)
        }
    }

    private fun hideTarget() {
        target?.let(windows::removeView)
        target = null
    }

    @SuppressLint("ClickableViewAccessibility")
    fun showSnoozeMenu() {
        hide()
        val panel = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(8), dp(16), dp(8), dp(8))
            background = GradientDrawable().apply {
                cornerRadius = dp(24).toFloat()
                setColor(PANEL)
                setStroke(dp(1), BORDER)
            }
        }
        panel.addView(
            TextView(context).apply {
                text = context.getString(R.string.snooze_title)
                setTextColor(TEXT)
                textSize = 17f
                typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
                setPadding(dp(16), 0, dp(16), dp(8))
            },
        )
        val choices = listOf(
            R.string.snooze_15_minutes to Snooze.MINUTES_15,
            R.string.snooze_hour to Snooze.HOUR,
            R.string.snooze_tomorrow to Snooze.TOMORROW,
            R.string.snooze_always_here to Snooze.ALWAYS_HERE,
        )
        for ((label, choice) in choices) {
            panel.addView(menuRow(label, TEXT) { closeMenu(); actions.onSnooze(choice) })
        }
        panel.addView(menuRow(R.string.snooze_cancel, EMBER) { closeMenu(); show() })
        // This window cannot get the Back key, so a touch anywhere else keeps the bubble too.
        panel.setOnTouchListener { _, event ->
            if (event.actionMasked == MotionEvent.ACTION_OUTSIDE) {
                closeMenu()
                show()
            }
            false
        }
        val layout = overlayParams(dp(296), WindowManager.LayoutParams.WRAP_CONTENT).apply {
            gravity = Gravity.CENTER
            flags = flags or WindowManager.LayoutParams.FLAG_WATCH_OUTSIDE_TOUCH
        }
        windows.addView(panel, layout)
        menu = panel
    }

    private fun menuRow(label: Int, color: Int, onClick: () -> Unit) = TextView(context).apply {
        text = context.getString(label)
        textSize = 16f
        setTextColor(color)
        setPadding(dp(16), dp(14), dp(16), dp(14))
        val shape = GradientDrawable().apply {
            cornerRadius = dp(14).toFloat()
            setColor(Color.WHITE)
        }
        background = RippleDrawable(ColorStateList.valueOf(RIPPLE), null, shape)
        isClickable = true
        setOnClickListener { onClick() }
    }

    private fun closeMenu() {
        menu?.let(windows::removeView)
        menu = null
    }

    fun destroy() {
        removeHint()
        hideTarget()
        closeMenu()
        hide()
    }

    private fun overlayParams(width: Int, height: Int) = WindowManager.LayoutParams(
        width,
        height,
        WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
        WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
            WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
        PixelFormat.TRANSLUCENT,
    )

    companion object {
        private const val TARGET_DP = 64
        private const val TARGET_MARGIN_DP = 96
        private const val HINT_MS = 3_000L
        private val TEXT = Color.parseColor("#F2F2F3")
        private val EMBER = Color.parseColor("#FF7A1A")
        private val PANEL = Color.parseColor("#F2141417")
        private val BORDER = Color.parseColor("#1FF2F2F3")
        private val RIPPLE = Color.parseColor("#33F2F2F3")
    }
}
