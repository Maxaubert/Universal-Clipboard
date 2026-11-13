package dev.uc.s7

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import android.util.Log

const val TAG = "S7"

object Clip {
    /**
     * Writes text to the clipboard and logs the result.
     * flags: "overlay" sets the SystemUI switch that makes the emulator show the preview overlay
     * like a phone does; "remote" sets EXTRA_IS_REMOTE_DEVICE; "sensitive" sets EXTRA_IS_SENSITIVE.
     */
    fun write(ctx: Context, where: String, text: String, flags: Set<String>) {
        val cm = ctx.getSystemService(ClipboardManager::class.java)
        val clip = ClipData.newPlainText("uc", text)
        val extras = PersistableBundle()
        if ("overlay" in flags) extras.putBoolean("com.android.systemui.SHOW_CLIPBOARD_OVERLAY", true)
        if ("remote" in flags && Build.VERSION.SDK_INT >= 33) extras.putBoolean(ClipDescription.EXTRA_IS_REMOTE_DEVICE, true)
        if ("sensitive" in flags) extras.putBoolean("android.content.extra.IS_SENSITIVE", true)
        if (!extras.isEmpty) clip.description.extras = extras
        try {
            cm.setPrimaryClip(clip)
            Log.i(TAG, "WRITE ok where=$where sdk=${Build.VERSION.SDK_INT} flags=$flags text=$text")
        } catch (t: Throwable) {
            Log.e(TAG, "WRITE failed where=$where sdk=${Build.VERSION.SDK_INT}: $t")
        }
    }

    fun read(ctx: Context, where: String): String? {
        val cm = ctx.getSystemService(ClipboardManager::class.java)
        return try {
            val has = cm.hasPrimaryClip()
            val clip = cm.primaryClip
            val text = clip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(ctx)?.toString()
            Log.i(TAG, "READ where=$where sdk=${Build.VERSION.SDK_INT} hasPrimaryClip=$has clip=${if (clip == null) "null" else "present"} text=$text")
            text
        } catch (t: Throwable) {
            Log.e(TAG, "READ failed where=$where: $t")
            null
        }
    }

    fun nativeCheck(where: String) {
        try {
            Log.i(TAG, "NATIVE jni add(40,2)=${Native.add(40, 2)} where=$where")
        } catch (t: Throwable) {
            Log.e(TAG, "NATIVE jni failed where=$where: $t")
        }
        try {
            Log.i(TAG, "NATIVE uniffi greet=${uniffi.uc_s7.greet("android")} info=${uniffi.uc_s7.coreInfo()} where=$where")
        } catch (t: Throwable) {
            Log.e(TAG, "NATIVE uniffi failed where=$where: $t")
        }
    }
}
