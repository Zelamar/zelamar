package com.zelamar.app

import android.app.Notification
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/**
 * Runs while a call goes on (ADR-0019), so Android keeps the microphone,
 * and the camera in a video call, when the screen goes off or another app
 * comes to the front. Its notification says so and leads back to the call.
 */
class CallService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val title = intent?.getStringExtra(EXTRA_TITLE).orEmpty().ifEmpty { "Zelamar" }
        val video = intent?.getBooleanExtra(EXTRA_VIDEO, false) ?: false
        val open = PendingIntent.getActivity(
            this,
            4,
            Intent(this, MainActivity::class.java)
                .putExtra(MainActivity.EXTRA_CALL, true)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, ZelamarPlatform.ONGOING)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
        val notification = builder
            .setSmallIcon(R.drawable.ic_stat_zelamar)
            .setColor(ZelamarPlatform.BRONZE)
            .setContentTitle(title)
            .setContentText(if (video) "Video call" else "Call")
            .setCategory(Notification.CATEGORY_CALL)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
                if (video) {
                    types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
                }
                startForeground(ZelamarPlatform.ONGOING_NOTIFICATION, notification, types)
            } else {
                startForeground(ZelamarPlatform.ONGOING_NOTIFICATION, notification)
            }
        } catch (error: Exception) {
            // Not allowed right now (no microphone permission yet, say):
            // the call still goes on while Zelamar is open.
            stopSelf()
        }
        return START_NOT_STICKY
    }

    companion object {
        const val EXTRA_TITLE = "com.zelamar.app.TITLE"
        const val EXTRA_VIDEO = "com.zelamar.app.VIDEO"
    }
}
