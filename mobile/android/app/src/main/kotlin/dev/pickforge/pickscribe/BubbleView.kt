package dev.pickforge.pickscribe

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.view.View
import android.view.animation.LinearInterpolator
import kotlin.math.PI
import kotlin.math.max
import kotlin.math.sin

/** The round bubble: a microphone at rest, live bars while listening, a pulse while thinking. */
class BubbleView(context: Context) : View(context) {
    enum class Mode { IDLE, LISTENING, THINKING }

    var mode = Mode.IDLE
        private set
    private var raw = false
    private var levelSource: () -> Float = { 0f }
    private val levels = FloatArray(BARS)
    private var phase = 0f

    private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = SURFACE }
    private val ring = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
    private val ink = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeCap = Paint.Cap.ROUND
    }
    private val box = RectF()

    private val ticker = ValueAnimator.ofFloat(0f, 1f).apply {
        duration = 1_000
        repeatCount = ValueAnimator.INFINITE
        interpolator = LinearInterpolator()
        addUpdateListener {
            phase = it.animatedValue as Float
            if (mode == Mode.LISTENING) {
                levels.copyInto(levels, 0, 1)
                levels[BARS - 1] = levelSource()
            }
            invalidate()
        }
    }

    fun idle() = setMode(Mode.IDLE)

    fun listening(raw: Boolean, level: () -> Float) {
        this.raw = raw
        levelSource = level
        levels.fill(0f)
        setMode(Mode.LISTENING)
    }

    fun thinking() = setMode(Mode.THINKING)

    private fun setMode(next: Mode) {
        mode = next
        if (next == Mode.IDLE) ticker.cancel() else if (!ticker.isStarted) ticker.start()
        invalidate()
    }

    override fun onDetachedFromWindow() {
        ticker.cancel()
        super.onDetachedFromWindow()
    }

    override fun onDraw(canvas: Canvas) {
        val size = width.coerceAtMost(height).toFloat()
        val cx = width / 2f
        val cy = height / 2f
        val radius = size / 2f - size * 0.06f
        canvas.drawCircle(cx, cy, radius, fill)
        val accent = if (raw) TEXT else EMBER
        ring.strokeWidth = size * 0.04f
        ring.color = if (mode == Mode.IDLE) MUTED else accent
        canvas.drawCircle(cx, cy, radius, ring)
        ink.strokeWidth = size * 0.07f
        when (mode) {
            Mode.IDLE -> drawMic(canvas, cx, cy, size)
            Mode.LISTENING -> drawBars(canvas, cx, cy, size, accent)
            Mode.THINKING -> drawPulse(canvas, cx, cy, radius, accent)
        }
    }

    private fun drawMic(canvas: Canvas, cx: Float, cy: Float, size: Float) {
        ink.color = TEXT
        val w = size * 0.12f
        val h = size * 0.2f
        box.set(cx - w, cy - h * 1.1f, cx + w, cy + h * 0.5f)
        canvas.drawRoundRect(box, w, w, ink)
        box.set(cx - w * 1.8f, cy - h * 0.6f, cx + w * 1.8f, cy + h * 0.9f)
        canvas.drawArc(box, 20f, 140f, false, ink)
        canvas.drawLine(cx, cy + h * 0.9f, cx, cy + h * 1.4f, ink)
    }

    private fun drawBars(canvas: Canvas, cx: Float, cy: Float, size: Float, accent: Int) {
        ink.color = accent
        val gap = size * 0.11f
        val start = cx - gap * (BARS - 1) / 2f
        for (i in 0 until BARS) {
            // Speech RMS sits well below 1, so the bars scale it up before clamping.
            val level = (levels[i] * 6f).coerceIn(0.08f, 1f)
            val half = max(size * 0.03f, level * size * 0.22f)
            val x = start + gap * i
            canvas.drawLine(x, cy - half, x, cy + half, ink)
        }
    }

    private fun drawPulse(canvas: Canvas, cx: Float, cy: Float, radius: Float, accent: Int) {
        val swell = (sin(phase * 2 * PI) * 0.5 + 0.5).toFloat()
        fill.color = accent
        fill.alpha = (60 + 120 * swell).toInt()
        canvas.drawCircle(cx, cy, radius * (0.25f + 0.2f * swell), fill)
        fill.color = SURFACE
    }

    companion object {
        private const val BARS = 5
        private val EMBER = Color.parseColor("#FF7A1A")
        private val SURFACE = Color.parseColor("#0F0F11")
        private val TEXT = Color.parseColor("#F2F2F3")
        private val MUTED = Color.parseColor("#6E6E75")
    }
}
