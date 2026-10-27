package com.zelamar.app

import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine

/**
 * The window. It shows the process's one engine ([ZelamarEngine]), which
 * may already be running: a call wake-up starts it without a window. The
 * engine outlives the window, so a closed window still hears calls while
 * Android keeps the process.
 */
class MainActivity : FlutterActivity() {
    override fun provideFlutterEngine(context: Context): FlutterEngine =
        ZelamarEngine.get(context)

    override fun shouldDestroyEngineWithHost(): Boolean = false

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        ZelamarPlatform(applicationContext).install(flutterEngine, this)
    }

    override fun cleanupFlutterEngine(flutterEngine: FlutterEngine) {
        // Without a window: nothing is in front, and no one to ask.
        ZelamarPlatform(applicationContext).install(flutterEngine, null)
        super.cleanupFlutterEngine(flutterEngine)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handleCall(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleCall(intent)
    }

    /** Opened for a ringing call: show it over the lock screen; answer if asked. */
    private fun handleCall(opened: Intent?) {
        val intent = opened ?: return
        if (!intent.getBooleanExtra(EXTRA_CALL, false)) {
            return
        }
        showOverLock(true)
        if (intent.getBooleanExtra(EXTRA_ACCEPT, false)) {
            ZelamarPlatform(applicationContext).stopRinging()
            ZelamarEngine.running()?.let { ZelamarEngine.tell(it, "accept") }
        }
        // Handled once: a recreated window must not answer again.
        intent.removeExtra(EXTRA_ACCEPT)
    }

    /**
     * Lets a call show over the lock screen and wake the screen, only while
     * a call rings or goes on; otherwise the lock screen hides everything.
     */
    fun showOverLock(on: Boolean) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
            setShowWhenLocked(on)
            setTurnScreenOn(on)
        } else {
            @Suppress("DEPRECATION")
            val flags = WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or
                WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON
            if (on) window.addFlags(flags) else window.clearFlags(flags)
        }
    }

    companion object {
        /** The window was opened for a ringing call. */
        const val EXTRA_CALL = "com.zelamar.app.CALL"

        /** ... and the user chose to answer it. */
        const val EXTRA_ACCEPT = "com.zelamar.app.ACCEPT"
    }
}
