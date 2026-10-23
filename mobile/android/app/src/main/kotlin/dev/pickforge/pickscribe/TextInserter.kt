package dev.pickforge.pickscribe

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.Bundle
import android.os.PersistableBundle
import android.view.accessibility.AccessibilityNodeInfo

/** Puts dictated text at the cursor of a text field in another app. */
object TextInserter {
    /**
     * Pastes through the clipboard, since the field's own paste handles rich editors and web
     * views. Fields that refuse to paste get the text merged in with ACTION_SET_TEXT. The field
     * must still have input focus, so text never lands in a field the user has left. After a
     * failure, or with no field, the text is on the clipboard to paste by hand.
     *
     * The text stays on the clipboard. Android lets only the focused app read the clipboard, so a
     * later clear could not tell our clip from one the user or an app copied since.
     */
    fun insert(context: Context, field: AccessibilityNodeInfo?, text: String): Boolean {
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        val inserted = field != null && field.refresh() && field.isFocused && field.isEditable &&
            !field.isPassword && insertAtCursor(clipboard, field, text)
        if (!inserted) copy(clipboard, text)
        return inserted
    }

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

    // ClipDescription.EXTRA_IS_SENSITIVE, which the SDK names only from Android 13.
    private const val EXTRA_IS_SENSITIVE = "android.content.extra.IS_SENSITIVE"
}
