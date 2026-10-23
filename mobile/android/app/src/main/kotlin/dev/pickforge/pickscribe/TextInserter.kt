package dev.pickforge.pickscribe

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import android.view.accessibility.AccessibilityNodeInfo

/** Puts dictated text at the cursor of a text field in another app. */
object TextInserter {
    private val main = Handler(Looper.getMainLooper())
    private var clipboard: ClipboardManager? = null
    private val clearClip = Runnable {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) clipboard?.clearPrimaryClip()
    }

    /**
     * Pastes through the clipboard, since the field's own paste handles rich editors and web
     * views. Fields that refuse to paste get the text merged in with ACTION_SET_TEXT. The field
     * must still have input focus, so text never lands in a field the user has left. After an
     * insert the clip is cleared; after a failure, or with no field, the text stays there to
     * paste by hand.
     */
    fun insert(context: Context, field: AccessibilityNodeInfo?, text: String): Boolean {
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        this.clipboard = clipboard
        main.removeCallbacks(clearClip)
        val inserted = field != null && field.refresh() && field.isFocused && field.isEditable &&
            !field.isPassword && insertAtCursor(clipboard, field, text)
        if (inserted) {
            // Web views read the clip after the paste action returns, so the clear waits.
            main.postDelayed(clearClip, CLEAR_DELAY_MS)
        } else {
            copy(clipboard, text)
        }
        return inserted
    }

    /**
     * Cancels a pending clear. Android lets only the focused app read the clipboard, so the clear
     * cannot check that the clip is still ours. Call this when the user may have copied something.
     */
    fun keepClip() = main.removeCallbacks(clearClip)

    private fun insertAtCursor(clipboard: ClipboardManager, field: AccessibilityNodeInfo, text: String): Boolean {
        val current = currentText(field)
        val (start, end) = selection(field, current.length)
        val spaced = withSpace(current, start, text)
        copy(clipboard, spaced)
        if (field.performAction(AccessibilityNodeInfo.ACTION_PASTE)) return true
        return setText(field, current, start, end, spaced)
    }

    private fun currentText(field: AccessibilityNodeInfo): String {
        val hint = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && field.isShowingHintText
        return if (hint) "" else field.text?.toString().orEmpty()
    }

    /** Adds a space before [text] when it would otherwise touch the word before the cursor. */
    internal fun withSpace(current: String, cursor: Int, text: String): String =
        if (cursor > 0 && !current[cursor - 1].isWhitespace() && text.firstOrNull()?.isWhitespace() == false) {
            " $text"
        } else {
            text
        }

    private fun copy(clipboard: ClipboardManager, text: String) {
        val clip = ClipData.newPlainText("PickScribe", text)
        // Asks keyboards to keep the clip out of clipboard history. Android does not enforce it.
        clip.description.extras = PersistableBundle().apply { putBoolean(EXTRA_IS_SENSITIVE, true) }
        clipboard.setPrimaryClip(clip)
    }

    private fun setText(field: AccessibilityNodeInfo, current: String, start: Int, end: Int, text: String): Boolean {
        val merged = current.substring(0, start) + text + current.substring(end)
        val args = Bundle().apply {
            putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, merged)
        }
        if (!field.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)) return false
        val cursor = start + text.length
        field.performAction(
            AccessibilityNodeInfo.ACTION_SET_SELECTION,
            Bundle().apply {
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, cursor)
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, cursor)
            },
        )
        return true
    }

    /** The selection inside [length], or the end of the text when the field reports none. */
    internal fun selection(field: AccessibilityNodeInfo, length: Int): Pair<Int, Int> =
        clampSelection(field.textSelectionStart, field.textSelectionEnd, length)

    internal fun clampSelection(start: Int, end: Int, length: Int): Pair<Int, Int> {
        if (start < 0 || end < 0 || start > length || end > length) return length to length
        return minOf(start, end) to maxOf(start, end)
    }

    private const val CLEAR_DELAY_MS = 1_000L

    // ClipDescription.EXTRA_IS_SENSITIVE, which the SDK names only from Android 13.
    private const val EXTRA_IS_SENSITIVE = "android.content.extra.IS_SENSITIVE"
}
