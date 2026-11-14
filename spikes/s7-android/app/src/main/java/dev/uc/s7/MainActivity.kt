package dev.uc.s7

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.widget.TextView

class MainActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(TextView(this).apply { text = "S7 spike. Service started."; textSize = 20f; setPadding(48, 160, 48, 48) })
        startForegroundService(Intent(this, ClipService::class.java))
        Clip.nativeCheck("activity")
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) Clip.read(this, "main-focus")
    }
}
