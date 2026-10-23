package dev.pickforge.pickscribe

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import java.io.ByteArrayOutputStream

/** Names and icons of other apps, for History and the blocked apps list. */
object Apps {
    /** One map per package: its label, or null when the app is gone, and a PNG icon. */
    fun describe(context: Context, packages: List<String>, iconPx: Int): List<Map<String, Any?>> {
        val pm = context.packageManager
        return packages.map { name ->
            try {
                val info = pm.getApplicationInfo(name, 0)
                mapOf(
                    "package" to name,
                    "label" to pm.getApplicationLabel(info).toString(),
                    "icon" to iconPng(pm, name, iconPx),
                )
            } catch (_: PackageManager.NameNotFoundException) {
                mapOf("package" to name, "label" to null, "icon" to null)
            }
        }
    }

    /** Every app with a launcher icon except this one, sorted by label. */
    fun launchable(context: Context): List<Map<String, String>> {
        val pm = context.packageManager
        val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        return pm.queryIntentActivities(launcher, 0)
            .map { it.activityInfo.applicationInfo }
            .filter { it.packageName != context.packageName }
            .distinctBy { it.packageName }
            .map { mapOf("package" to it.packageName, "label" to pm.getApplicationLabel(it).toString()) }
            .sortedBy { it.getValue("label").lowercase() }
    }

    private fun iconPng(pm: PackageManager, name: String, size: Int): ByteArray {
        val drawable = pm.getApplicationIcon(name)
        val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, size, size)
        drawable.draw(Canvas(bitmap))
        return ByteArrayOutputStream().use { out ->
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
            bitmap.recycle()
            out.toByteArray()
        }
    }
}
