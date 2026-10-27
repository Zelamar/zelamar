package com.zelamar.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** "Decline" on a ringing call's notification: turned down without opening Zelamar. */
class CallActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != DECLINE) {
            return
        }
        ZelamarPlatform(context.applicationContext).stopRinging()
        ZelamarEngine.running()?.let { ZelamarEngine.tell(it, "decline") }
    }

    companion object {
        const val DECLINE = "com.zelamar.app.DECLINE"
    }
}
