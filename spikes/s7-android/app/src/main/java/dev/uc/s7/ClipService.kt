package dev.uc.s7

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log

/**
 * Foreground service of type connectedDevice. Stands in for the real sync service: a broadcast
 * plays the role of "a clip arrived from the network". Commands (adb shell am broadcast -p dev.uc.s7):
 *   dev.uc.s7.WRITE   --es text T [--es flags overlay,remote,sensitive] [--ei delayMs N]
 *   dev.uc.s7.READ    try to read the clipboard from the service (background)
 *   dev.uc.s7.NATIVE  call the Rust library from the service
 *   dev.uc.s7.TAP     post the fallback "Tap to copy" notification for --es text T
 */
class ClipService : Service() {
    private val main = Handler(Looper.getMainLooper())

    private val rx = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            when (i.action) {
                "dev.uc.s7.WRITE" -> {
                    val text = i.getStringExtra("text") ?: "spike"
                    val flags = (i.getStringExtra("flags") ?: "").split(',').filter { it.isNotBlank() }.toSet()
                    val delay = i.getIntExtra("delayMs", 0).toLong()
                    main.postDelayed({ Clip.write(this@ClipService, "service", text, flags) }, delay)
                }
                "dev.uc.s7.READ" -> Clip.read(this@ClipService, "service")
                "dev.uc.s7.NATIVE" -> Clip.nativeCheck("service")
                "dev.uc.s7.TAP" -> postTapToCopy(i.getStringExtra("text") ?: "spike")
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel("svc", "Sync service", NotificationManager.IMPORTANCE_MIN))
        nm.createNotificationChannel(NotificationChannel("clip", "Incoming clips", NotificationManager.IMPORTANCE_HIGH))
        val n = Notification.Builder(this, "svc")
            .setSmallIcon(R.drawable.ic_clip)
            .setContentTitle("Connected to your devices")
            .setOngoing(true)
            .build()
        try {
            startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
            Log.i(TAG, "FGS started type=connectedDevice sdk=${Build.VERSION.SDK_INT}")
        } catch (t: Throwable) {
            Log.e(TAG, "FGS start failed: $t")
        }
        val f = IntentFilter().apply {
            addAction("dev.uc.s7.WRITE"); addAction("dev.uc.s7.READ"); addAction("dev.uc.s7.NATIVE"); addAction("dev.uc.s7.TAP")
        }
        if (Build.VERSION.SDK_INT >= 33) registerReceiver(rx, f, Context.RECEIVER_EXPORTED) else registerReceiver(rx, f)
    }

    private fun postTapToCopy(text: String) {
        val i = Intent(this, FocusActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).putExtra("from", "notification").putExtra("write", text)
        val pi = PendingIntent.getActivity(this, 2, i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = Notification.Builder(this, "clip")
            .setSmallIcon(R.drawable.ic_clip)
            .setContentTitle("Clip from your PC")
            .setContentText("Tap to copy: $text")
            .setContentIntent(pi)
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(2, n)
        Log.i(TAG, "TAP notification posted")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int) = START_STICKY
    override fun onBind(intent: Intent?): IBinder? = null
    override fun onDestroy() { unregisterReceiver(rx); super.onDestroy() }
}
