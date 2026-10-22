package dev.pickforge.pickscribe

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Bundle
import android.view.accessibility.AccessibilityNodeInfo

/** Puts dictated text at the cursor of a text field in another app. */
object TextInserter {
    /**
     * Pastes through the clipboard, since the field's own paste handles rich editors and web
     * views. Fields that refuse to paste get the text merged in with ACTION_SET_TEXT. The text
     * stays on the clipboard either way, so a failed insert can still be pasted by hand.
     */
    fun insert(context: Context, field: AccessibilityNodeInfo, text: String): Boolean {
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        clipboard.setPrimaryClip(ClipData.newPlainText("PickScribe", text))
        if (!field.refresh()) return false
        if (field.performAction(AccessibilityNodeInfo.ACTION_PASTE)) return true
        return setText(field, text)
    }

    private fun setText(field: AccessibilityNodeInfo, text: String): Boolean {
        val current = if (field.isShowingHintText) "" else field.text?.toString().orEmpty()
        val (start, end) = selection(field, current.length)
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
}
