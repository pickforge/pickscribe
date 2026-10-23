package dev.pickforge.pickscribe

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.RadialGradient
import android.graphics.RectF
import android.graphics.Shader
import android.graphics.SweepGradient
import android.view.View
import android.view.animation.LinearInterpolator
import kotlin.math.PI
import kotlin.math.log10
import kotlin.math.max
import kotlin.math.sin

/**
 * The round bubble: the PickScribe mark at rest, bars that follow the voice while listening, and
 * an arc that turns around the rim while thinking.
 */
class BubbleView(context: Context) : View(context) {
    enum class Mode { IDLE, LISTENING, THINKING }

    var mode = Mode.IDLE
        private set
    private var raw = false
    private var levelSource: () -> Float = { 0f }
    private var loud = 0f
    private var phase = 0f

    // Geometry, set in onSizeChanged. The orb leaves a margin for its shadow and glow.
    private var size = 0f
    private var cx = 0f
    private var cy = 0f
    private var radius = 0f
    private var unit = 0f

    private val glow = Paint(Paint.ANTI_ALIAS_FLAG)
    private val shadow = Paint(Paint.ANTI_ALIAS_FLAG)
    private val body = Paint(Paint.ANTI_ALIAS_FLAG)
    private val rim = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
    private var shine: Shader? = null
    private val ink = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeCap = Paint.Cap.ROUND
    }
    private val fill = Paint(Paint.ANTI_ALIAS_FLAG)
    private val spark = Paint(Paint.ANTI_ALIAS_FLAG)
    private val arc = Paint(Paint.ANTI_ALIAS_FLAG).apply {
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
                val target = loudness(levelSource())
                // Rises fast and falls slowly, so the bars follow syllables without flicker.
                loud += (target - loud) * if (target > loud) 0.5f else 0.15f
            }
            invalidate()
        }
    }

    fun idle() = setMode(Mode.IDLE)

    fun listening(raw: Boolean, level: () -> Float) {
        this.raw = raw
        levelSource = level
        loud = 0f
        shade()
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

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        size = minOf(w, h).toFloat()
        cx = w / 2f
        cy = h / 2f
        radius = size * 0.4f
        unit = radius / 10f
        shadow.shader = RadialGradient(
            cx, cy + radius * 0.1f, radius * SHADOW,
            intArrayOf(alpha(Color.BLACK, 0.5f), alpha(Color.BLACK, 0.32f), Color.TRANSPARENT),
            floatArrayOf(0f, 0.75f, 1f),
            Shader.TileMode.CLAMP,
        )
        body.shader = LinearGradient(
            0f, cy - radius, 0f, cy + radius, BODY_TOP, BODY_BOTTOM, Shader.TileMode.CLAMP,
        )
        shine = LinearGradient(
            0f, cy - radius, 0f, cy + radius,
            // The lower edge stays visible, so the orb keeps its outline over a black app.
            alpha(TEXT, 0.3f), alpha(TEXT, 0.14f),
            Shader.TileMode.CLAMP,
        )
        spark.shader = RadialGradient(
            dotX(), cy, DOT * unit * 2.4f,
            alpha(EMBER, 0.4f), Color.TRANSPARENT,
            Shader.TileMode.CLAMP,
        )
        rim.strokeWidth = max(1f, size * 0.022f)
        arc.strokeWidth = max(1.5f, size * 0.035f)
        ink.strokeWidth = unit * 1.3f
        shade()
    }

    // The glow and the arc take the accent, which changes with raw mode.
    private fun shade() {
        if (size == 0f) return
        val accent = accent()
        glow.shader = RadialGradient(
            cx, cy, size / 2f,
            intArrayOf(alpha(accent, 0.55f), alpha(accent, 0.4f), Color.TRANSPARENT),
            floatArrayOf(0f, 0.78f, 1f),
            Shader.TileMode.CLAMP,
        )
        // The tail fades in behind the head, and the head's round cap stays solid.
        arc.shader = SweepGradient(
            cx, cy,
            intArrayOf(Color.TRANSPARENT, accent, accent, Color.TRANSPARENT),
            floatArrayOf(0f, ARC_TURN, 0.85f, 1f),
        )
    }

    override fun onDraw(canvas: Canvas) {
        val accent = accent()
        if (mode == Mode.LISTENING) {
            glow.alpha = ((0.25f + 0.75f * loud) * 255).toInt()
            canvas.drawCircle(cx, cy, size / 2f, glow)
        }
        canvas.drawCircle(cx, cy + radius * 0.1f, radius * SHADOW, shadow)
        canvas.drawCircle(cx, cy, radius, body)
        if (mode == Mode.LISTENING) {
            rim.shader = null
            rim.color = alpha(accent, 0.9f)
        } else {
            rim.shader = shine
            rim.color = Color.WHITE
        }
        canvas.drawCircle(cx, cy, radius - rim.strokeWidth / 2f, rim)
        when (mode) {
            Mode.IDLE -> drawMark(canvas)
            Mode.LISTENING -> drawBars(canvas, accent)
            Mode.THINKING -> drawThinking(canvas)
        }
    }

    // The PickScribe mark: three bars, then the ember dot.
    private fun drawMark(canvas: Canvas) {
        ink.color = alpha(TEXT, 0.92f)
        for ((i, half) in MARK.withIndex()) {
            val x = cx + (MARK_START + i * GAP) * unit
            canvas.drawLine(x, cy - half * unit, x, cy + half * unit, ink)
        }
        canvas.drawCircle(dotX(), cy, DOT * unit * 2.4f, spark)
        fill.color = EMBER
        canvas.drawCircle(dotX(), cy, DOT * unit, fill)
    }

    private fun drawBars(canvas: Canvas, accent: Int) {
        ink.color = accent
        val low = unit * 0.65f
        val high = radius * 0.48f
        for (i in SHAPE.indices) {
            // Each bar sways at its own pace, so the bars never move in lockstep.
            val sway = 0.72f + 0.28f * sin(2 * PI * phase * SWAY[i] + i * 1.3).toFloat()
            val half = low + (high - low) * loud * SHAPE[i] * sway
            val x = cx + (i - SHAPE.size / 2) * GAP * unit
            canvas.drawLine(x, cy - half, x, cy + half, ink)
        }
    }

    // An arc turns around the rim once a second while three dots rise in a wave.
    private fun drawThinking(canvas: Canvas) {
        val r = radius - arc.strokeWidth / 2f
        box.set(cx - r, cy - r, cx + r, cy + r)
        canvas.save()
        canvas.rotate(phase * 360f, cx, cy)
        canvas.drawArc(box, 0f, ARC_TURN * 360f, false, arc)
        canvas.restore()
        for (i in 0 until 3) {
            val wave = (sin(2 * PI * (phase * 2 - i * 0.18)) * 0.5 + 0.5).toFloat()
            fill.color = alpha(TEXT, 0.3f + 0.7f * wave)
            canvas.drawCircle(cx + (i - 1) * 3 * unit, cy - wave * unit * 0.8f, unit * 1.05f, fill)
        }
    }

    private fun dotX() = cx + (MARK_START + (MARK.size - 1) * GAP + DOT_OFFSET) * unit

    private fun accent() = if (raw) TEXT else EMBER

    // Maps the microphone level from room noise at -50 dB to close speech at -15 dB.
    private fun loudness(rms: Float) =
        ((20 * log10(max(rms, 1e-5f)) + 50f) / 35f).coerceIn(0f, 1f)

    private fun alpha(color: Int, alpha: Float) = (color and 0xFFFFFF) or ((alpha * 255).toInt() shl 24)

    companion object {
        // In units of a tenth of the orb's radius.
        private val MARK = floatArrayOf(1.5f, 3.3f, 2.1f)
        private const val MARK_START = -4.7f
        private const val GAP = 2.4f
        private const val DOT = 1.75f
        private const val DOT_OFFSET = 3.5f

        private val SHAPE = floatArrayOf(0.45f, 0.75f, 1f, 0.75f, 0.45f)
        private val SWAY = intArrayOf(3, 2, 4, 2, 3)
        private const val ARC_TURN = 0.7f

        // The shadow's radius, so that it fades out before the bottom edge of the view.
        private const val SHADOW = 1.15f

        private val EMBER = Color.parseColor("#FF7A1A")
        private val TEXT = Color.parseColor("#F2F2F3")
        private val BODY_TOP = Color.parseColor("#26262B")
        private val BODY_BOTTOM = Color.parseColor("#0B0B0D")
    }
}
