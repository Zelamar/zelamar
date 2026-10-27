package com.zelamar.app

import android.os.Handler
import android.os.Looper
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

/**
 * A call wake-up (ADR-0019): no words, no sender, only "fetch your mail
 * now". It starts Zelamar without a window, or wakes the running one; the
 * app then opens what arrived and rings only for a real call from someone
 * the user talks to. Anyone can send a wake-up, so nothing rings on the
 * wake-up alone. Message wake-ups are shown by Android itself (ADR-0017).
 */
class ZelamarMessagingService : FirebaseMessagingService() {
    override fun onMessageReceived(message: RemoteMessage) {
        if (message.data["wake"] != "call") {
            return
        }
        Handler(Looper.getMainLooper()).post {
            val running = ZelamarEngine.running()
            if (running == null) {
                // Starting the app fetches the mail.
                ZelamarEngine.get(applicationContext)
            } else {
                ZelamarEngine.tell(running, "wake")
            }
        }
    }

    override fun onNewToken(token: String) {
        // The app leaves its token with the mailbox each time it starts.
    }
}
