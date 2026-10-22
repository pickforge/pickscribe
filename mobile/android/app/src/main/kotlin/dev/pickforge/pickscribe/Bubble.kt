package dev.pickforge.pickscribe

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowManager
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import kotlin.math.abs
import kotlin.math.hypot

/**
 * Owns the bubble window and its gestures. A tap toggles dictation, a long press starts a raw
 * one, and dropping the bubble on the X at the bottom opens the snooze menu.
 */
class Bubble(
    private val context: Context,
    private val prefs: BubblePrefs,
    private val actions: Actions,
) {
    interface Actions {
        fun onTap()
        fun onLongPress()
        fun onSnooze(choice: Snooze)
    }

    enum class Snooze { MINUTES_15, HOUR, TOMORROW, ALWAYS_HERE }

    val view = BubbleView(context)
    private val windows = context.getSystemService(WindowManager::class.java)
    private val density = context.resources.displayMetrics.density
    private val params = overlayParams(sizePx(), sizePx()).apply { gravity = Gravity.TOP or Gravity.START }
    private var shown = false
    private var target: View? = null
    private var menu: View? = null

    init {
        attachGestures()
    }

    private fun sizePx() = (prefs.sizeDp * density).toInt()

    fun show() {
        if (shown || menu != null) return
        applyPrefs()
        windows.addView(view, params)
        shown = true
    }

    fun hide() {
        if (!shown) return
        windows.removeView(view)
        shown = false
    }

    /** Applies size and opacity changes from the settings screen. */
    fun applyPrefs() {
        params.width = sizePx()
        params.height = sizePx()
        view.alpha = prefs.opacity
        val saved = prefs.position
        val screen = context.resources.displayMetrics
        params.x = saved?.first ?: (screen.widthPixels - params.width - (8 * density).toInt())
        params.y = saved?.second ?: (screen.heightPixels * 0.42f).toInt()
        if (shown) windows.updateViewLayout(view, params)
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
            pressedLong = true
            actions.onLongPress()
        }
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
                        showTarget()
                    }
                    if (dragging) {
                        params.x = startX + dx.toInt()
                        params.y = startY + dy.toInt()
                        windows.updateViewLayout(view, params)
                    }
                }
                MotionEvent.ACTION_UP -> {
                    view.removeCallbacks(longPressCheck)
                    when {
                        dragging -> finishDrag()
                        !pressedLong -> actions.onTap()
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
            val saved = prefs.position
            params.x = saved?.first ?: params.x
            params.y = saved?.second ?: params.y
            hide()
            showMenu()
        } else {
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
        val size = (TARGET_DP * density).toInt()
        val x = TextView(context).apply {
            text = "✕"
            textSize = 22f
            gravity = Gravity.CENTER
            setTextColor(Color.parseColor("#F2F2F3"))
            background = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(Color.parseColor("#CC0F0F11"))
            }
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
    }

    private fun hideTarget() {
        target?.let(windows::removeView)
        target = null
    }

    private fun showMenu() {
        val pad = (16 * density).toInt()
        val panel = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad, pad, pad)
            background = GradientDrawable().apply {
                cornerRadius = 20 * density
                setColor(Color.parseColor("#0F0F11"))
            }
        }
        panel.addView(
            TextView(context).apply {
                text = context.getString(R.string.snooze_title)
                setTextColor(Color.parseColor("#F2F2F3"))
                textSize = 16f
                setPadding(0, 0, 0, pad / 2)
            },
        )
        val choices = listOf(
            R.string.snooze_15_minutes to Snooze.MINUTES_15,
            R.string.snooze_hour to Snooze.HOUR,
            R.string.snooze_tomorrow to Snooze.TOMORROW,
            R.string.snooze_always_here to Snooze.ALWAYS_HERE,
        )
        for ((label, choice) in choices) {
            panel.addView(menuButton(label) { closeMenu(); actions.onSnooze(choice) })
        }
        panel.addView(menuButton(R.string.snooze_cancel) { closeMenu(); show() })
        val layout = overlayParams(
            (280 * density).toInt(),
            WindowManager.LayoutParams.WRAP_CONTENT,
        ).apply { gravity = Gravity.CENTER }
        windows.addView(panel, layout)
        menu = panel
    }

    private fun menuButton(label: Int, onClick: () -> Unit) = Button(context).apply {
        text = context.getString(label)
        isAllCaps = false
        setOnClickListener { onClick() }
    }

    private fun closeMenu() {
        menu?.let(windows::removeView)
        menu = null
    }

    fun destroy() {
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
    }
}
