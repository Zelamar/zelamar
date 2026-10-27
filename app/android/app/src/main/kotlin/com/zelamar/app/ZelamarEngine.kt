package com.zelamar.app

import android.content.Context
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.embedding.engine.FlutterEngineCache
import io.flutter.embedding.engine.dart.DartExecutor
import io.flutter.plugin.common.MethodChannel

/**
 * The one Flutter engine of the process, and with it the one session over
 * the encrypted store. The window uses it, and so does a call wake-up that
 * arrives while the app is closed (ADR-0019): the wake-up starts it without
 * a window, the app fetches its mail, and rings if a real call is there.
 * Two engines would mean two sessions over one store, which must never be.
 */
object ZelamarEngine {
    private const val ID = "zelamar"

    /** Platform features the Dart side calls ("zelamar/…" channels). */
    private const val CALLS_CHANNEL = "zelamar/calls"

    /** The running engine, or `null` when the app has not started. */
    fun running(): FlutterEngine? = FlutterEngineCache.getInstance().get(ID)

    /** The running engine, started (without a window) if need be. Main thread only. */
    fun get(context: Context): FlutterEngine {
        running()?.let { return it }
        val app = context.applicationContext
        val engine = FlutterEngine(app)
        ZelamarPlatform(app).install(engine, null)
        engine.dartExecutor.executeDartEntrypoint(DartExecutor.DartEntrypoint.createDefault())
        FlutterEngineCache.getInstance().put(ID, engine)
        return engine
    }

    /** Tells the app about a call: `wake`, `accept` or `decline`. */
    fun tell(engine: FlutterEngine, what: String) {
        MethodChannel(engine.dartExecutor.binaryMessenger, CALLS_CHANNEL).invokeMethod(what, null)
    }
}
