package dev.uc.s7

import android.app.Activity
import android.os.Bundle
import android.os.SystemClock
import android.util.Log

/** Opened by the tile. Tries to read in onCreate and onResume (expected to fail), then with window focus. */
class FocusActivity : Activity() {
    private var t0 = 0L

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        t0 = SystemClock.elapsedRealtime()
        Log.i(TAG, "FOCUS onCreate from=${intent.getStringExtra("from")}")
        intent.getStringExtra("write")?.let { Clip.write(this, "focus-activity", it, emptySet()) }
        Clip.read(this, "focus-onCreate")
    }

    override fun onResume() {
        super.onResume()
        Clip.read(this, "focus-onResume")
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            Clip.read(this, "focus-windowFocus")
            Log.i(TAG, "FOCUS done in ${SystemClock.elapsedRealtime() - t0} ms")
            finish()
        }
    }
}
