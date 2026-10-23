package dev.pickforge.pickscribe

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.Build
import kotlin.math.sqrt

/** Records 16 kHz mono float audio, the format Whisper takes, into memory only. */
class Recorder {
    private var record: AudioRecord? = null
    private var thread: Thread? = null
    private val chunks = mutableListOf<FloatArray>()

    // How many chunks takeNew or stop has already returned.
    private var taken = 0

    /** Loudness of the latest chunk, from 0 to 1, for the bubble's waveform. */
    @Volatile
    var level = 0f
        private set

    /**
     * True when the last recording got only silence because a call or another app held the
     * microphone. From Android 10, the system silences a second recorder instead of refusing it.
     */
    var silenced = false
        private set

    /** Samples in the last recording. */
    var length = 0
        private set

    // The callers check RECORD_AUDIO before they start a recording.
    @SuppressLint("MissingPermission")
    fun start() {
        check(record == null) { "already recording" }
        val minimum = AudioRecord.getMinBufferSize(RATE, CHANNEL, ENCODING)
        val audio = AudioRecord(
            MediaRecorder.AudioSource.VOICE_RECOGNITION, RATE, CHANNEL, ENCODING,
            maxOf(minimum, CHUNK * Float.SIZE_BYTES * 4),
        )
        if (audio.state != AudioRecord.STATE_INITIALIZED) {
            audio.release()
            error("the microphone is unavailable")
        }
        synchronized(chunks) {
            chunks.clear()
            taken = 0
        }
        audio.startRecording()
        if (audio.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
            audio.release()
            error("the microphone is busy")
        }
        record = audio
        thread = Thread({ pump(audio) }, "pickscribe-recorder").also { it.start() }
    }

    /** Returns the samples recorded since the last call, so they can stream while recording. */
    fun takeNew(): FloatArray = synchronized(chunks) { takeRest() }

    /** Stops the recording and returns the samples that [takeNew] has not returned yet. */
    fun stop(): FloatArray {
        val audio = record ?: return FloatArray(0)
        record = null
        val flagged = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
            audio.activeRecordingConfiguration?.isClientSilenced == true
        audio.stop()
        thread?.join()
        thread = null
        audio.release()
        level = 0f
        return synchronized(chunks) {
            length = chunks.sumOf { it.size }
            silenced = flagged && chunks.all { chunk -> chunk.all { it == 0f } }
            val rest = takeRest()
            chunks.clear()
            rest
        }
    }

    // Callers hold the chunks lock.
    private fun takeRest(): FloatArray {
        val fresh = chunks.subList(taken, chunks.size)
        val out = FloatArray(fresh.sumOf { it.size })
        var offset = 0
        for (chunk in fresh) {
            chunk.copyInto(out, offset)
            offset += chunk.size
        }
        taken = chunks.size
        return out
    }

    private fun pump(audio: AudioRecord) {
        while (true) {
            val buffer = FloatArray(CHUNK)
            val read = audio.read(buffer, 0, CHUNK, AudioRecord.READ_BLOCKING)
            if (read <= 0) break
            val chunk = if (read == CHUNK) buffer else buffer.copyOf(read)
            synchronized(chunks) { chunks.add(chunk) }
            level = rms(chunk)
        }
    }

    private fun rms(chunk: FloatArray): Float {
        var sum = 0.0
        for (sample in chunk) sum += sample * sample
        return sqrt(sum / chunk.size).toFloat().coerceIn(0f, 1f)
    }

    companion object {
        const val RATE = 16_000
        private const val CHANNEL = AudioFormat.CHANNEL_IN_MONO
        private const val ENCODING = AudioFormat.ENCODING_PCM_FLOAT

        // 100 ms per read keeps the waveform lively.
        private const val CHUNK = RATE / 10
    }
}
