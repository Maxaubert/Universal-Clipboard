package dev.uc.s7

import android.app.PendingIntent
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.util.Log

class ClipTile : TileService() {
    override fun onStartListening() {
        qsTile?.apply { state = Tile.STATE_ACTIVE; updateTile() }
    }

    override fun onClick() {
        Log.i(TAG, "TILE onClick sdk=${Build.VERSION.SDK_INT}")
        val i = Intent(this, FocusActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).putExtra("from", "tile")
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(PendingIntent.getActivity(this, 1, i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(i)
        }
    }
}
