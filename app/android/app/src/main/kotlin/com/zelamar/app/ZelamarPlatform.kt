package com.zelamar.app

import android.Manifest
import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Person
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.Build
import android.os.PersistableBundle
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * What the Dart side asks of Android, with or without a window:
 * - notifications for new messages and calls ("zelamar/notify",
 *   lib/src/messenger/notifier.dart);
 * - the Firebase token for push wake-ups ("zelamar/push", ADR-0017);
 * - the master key, sealed with the Android Keystore ("zelamar/keystore",
 *   lib/src/messenger/os_vault.dart): a hardware-backed AES-256-GCM key that
 *   never leaves the device. Sealed bytes are `IV (12) || ciphertext || tag`;
 * - copying a secret marked sensitive ("zelamar/clipboard").
 */
class ZelamarPlatform(private val context: Context) {

    /** Answers the Dart side on [engine]; [activity] is the window, if one is open. */
    fun install(engine: FlutterEngine, activity: Activity?) {
        val messenger = engine.dartExecutor.binaryMessenger
        MethodChannel(messenger, KEYSTORE_CHANNEL).setMethodCallHandler { call, result ->
            try {
                val alias = requireNotNull(call.argument<String>("alias"))
                val data = requireNotNull(call.argument<ByteArray>("data"))
                when (call.method) {
                    "seal" -> result.success(seal(alias, data))
                    "open" -> result.success(open(alias, data))
                    else -> result.notImplemented()
                }
            } catch (error: Exception) {
                result.error("keystore", error.message ?: error.javaClass.simpleName, null)
            }
        }
        MethodChannel(messenger, PUSH_CHANNEL).setMethodCallHandler { call, result ->
            when (call.method) {
                "token" -> pushToken { result.success(it) }
                else -> result.notImplemented()
            }
        }
        MethodChannel(messenger, CLIPBOARD_CHANNEL).setMethodCallHandler { call, result ->
            when (call.method) {
                "copySecret" -> result.success(copySecret(call.arguments as? String))
                else -> result.notImplemented()
            }
        }
        createChannels()
        MethodChannel(messenger, NOTIFY_CHANNEL).setMethodCallHandler { call, result ->
            when (call.method) {
                "show" -> {
                    notify(
                        call.argument<String>("title").orEmpty(),
                        call.argument<String>("body").orEmpty(),
                    )
                    result.success(null)
                }
                "inFront" -> result.success(activity?.hasWindowFocus() ?: false)
                "requestPermission" -> {
                    activity?.let(::requestNotificationPermission)
                    result.success(null)
                }
                "ring" -> {
                    ring(
                        call.argument<String>("title").orEmpty(),
                        call.argument<Boolean>("video") ?: false,
                    )
                    result.success(null)
                }
                "stopRinging" -> {
                    stopRinging()
                    result.success(null)
                }
                "callOngoing" -> {
                    callOngoing(
                        call.argument<Boolean>("on") ?: false,
                        call.argument<String>("title").orEmpty(),
                        call.argument<Boolean>("video") ?: false,
                    )
                    (activity as? MainActivity)?.showOverLock(
                        call.argument<Boolean>("on") ?: false,
                    )
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        }
    }

    private fun manager() =
        context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

    private fun requestNotificationPermission(activity: Activity) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            activity.requestPermissions(
                arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                PERMISSION_REQUEST,
            )
        }
    }

    /**
     * This device's Firebase token, or `null` when the build has no Firebase
     * app or the token cannot be had right now.
     */
    private fun pushToken(done: (String?) -> Unit) {
        if (FirebaseApp.getApps(context).isEmpty()) {
            done(null)
            return
        }
        try {
            FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
                done(if (task.isSuccessful) task.result else null)
            }
        } catch (error: Exception) {
            done(null)
        }
    }

    /**
     * The channels: messages (push wake-ups use it too); calls, which ring
     * with the phone's ringtone until answered; and the quiet one that
     * says a call is going on.
     */
    private fun createChannels() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            return
        }
        val manager = manager()
        manager.createNotificationChannel(
            NotificationChannel(
                MESSAGES,
                "Messages",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply { description = "New messages" },
        )
        manager.createNotificationChannel(
            NotificationChannel(
                CALLS,
                "Calls",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "Incoming calls"
                setSound(
                    RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE),
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                        .build(),
                )
                enableVibration(true)
                lockscreenVisibility = Notification.VISIBILITY_PUBLIC
            },
        )
        manager.createNotificationChannel(
            NotificationChannel(
                ONGOING,
                "Calls in progress",
                NotificationManager.IMPORTANCE_LOW,
            ).apply { description = "A call is going on" },
        )
    }

    private fun builder(channel: String): Notification.Builder =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(context, channel)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(context).setPriority(Notification.PRIORITY_HIGH)
        }

    private fun openApp(code: Int, call: Boolean = false, accept: Boolean = false): PendingIntent =
        PendingIntent.getActivity(
            context,
            code,
            Intent(context, MainActivity::class.java)
                .putExtra(MainActivity.EXTRA_CALL, call)
                .putExtra(MainActivity.EXTRA_ACCEPT, accept)
                .addFlags(
                    Intent.FLAG_ACTIVITY_NEW_TASK or
                        Intent.FLAG_ACTIVITY_SINGLE_TOP or
                        Intent.FLAG_ACTIVITY_CLEAR_TOP,
                ),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

    /** One notification per sender would be next; for now, the latest message. */
    private fun notify(title: String, body: String) {
        createChannels()
        val open = openApp(0)
        val notification = builder(MESSAGES)
            .setSmallIcon(R.drawable.ic_stat_zelamar)
            .setColor(BRONZE)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body))
            .setContentIntent(open)
            .setAutoCancel(true)
            .setCategory(Notification.CATEGORY_MESSAGE)
            // A locked screen that hides private content shows only what a
            // push wake-up shows: no sender, no text.
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(publicNotification(open))
            .build()
        manager().notify(MESSAGE_NOTIFICATION, notification)
    }

    /** What a locked screen shows instead of a message. */
    private fun publicNotification(open: PendingIntent): Notification =
        builder(MESSAGES)
            .setSmallIcon(R.drawable.ic_stat_zelamar)
            .setColor(BRONZE)
            .setContentTitle("Zelamar")
            .setContentText("New message")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()

    /**
     * A call rings while Zelamar is out of sight: the ringtone until it is
     * answered, and the call over the lock screen where Android allows it.
     */
    private fun ring(title: String, video: Boolean) {
        createChannels()
        val open = openApp(1, call = true)
        val accept = openApp(2, call = true, accept = true)
        val decline = PendingIntent.getBroadcast(
            context,
            3,
            Intent(context, CallActionReceiver::class.java)
                .setAction(CallActionReceiver.DECLINE),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = builder(CALLS)
            .setSmallIcon(R.drawable.ic_stat_zelamar)
            .setColor(BRONZE)
            .setContentTitle(title)
            .setContentText(if (video) "Incoming video call" else "Incoming call")
            .setCategory(Notification.CATEGORY_CALL)
            .setContentIntent(open)
            .setFullScreenIntent(open, true)
            .setOngoing(true)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            // The caller gives up after 45 seconds.
            builder.setTimeoutAfter(RING_MS)
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            builder.setStyle(
                Notification.CallStyle.forIncomingCall(
                    Person.Builder().setName(title).setImportant(true).build(),
                    decline,
                    accept,
                ),
            )
        } else {
            builder
                .addAction(Notification.Action.Builder(null, "Decline", decline).build())
                .addAction(Notification.Action.Builder(null, "Accept", accept).build())
        }
        val notification = builder.build()
        notification.flags = notification.flags or Notification.FLAG_INSISTENT
        manager().notify(CALL_NOTIFICATION, notification)
    }

    fun stopRinging() {
        manager().cancel(CALL_NOTIFICATION)
    }

    /**
     * Keeps the microphone (and camera) working while the screen is off or
     * another app is in front, for as long as the call goes on.
     */
    private fun callOngoing(on: Boolean, title: String, video: Boolean) {
        val intent = Intent(context, CallService::class.java)
            .putExtra(CallService.EXTRA_TITLE, title)
            .putExtra(CallService.EXTRA_VIDEO, video)
        if (!on) {
            context.stopService(intent)
            return
        }
        stopRinging()
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        } catch (error: Exception) {
            // Not allowed right now: the call goes on while Zelamar is open.
        }
    }

    /**
     * Copies a secret, such as the recovery key, marked sensitive: Android
     * shows dots instead of it in the clipboard preview and keyboards do not
     * suggest it.
     */
    private fun copySecret(text: String?): Boolean {
        if (text == null) {
            return false
        }
        val clip = ClipData.newPlainText("Zelamar", text)
        clip.description.extras = PersistableBundle().apply {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
            } else {
                putBoolean("android.content.extra.IS_SENSITIVE", true)
            }
        }
        (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager)
            .setPrimaryClip(clip)
        return true
    }

    private fun keyStore(): KeyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }

    /** The existing key for [alias], creating it only when sealing. */
    private fun key(alias: String, create: Boolean): SecretKey {
        (keyStore().getKey(alias, null) as? SecretKey)?.let { return it }
        check(create) { "no key named $alias on this device" }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(
                alias,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    private fun seal(alias: String, secret: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key(alias, create = true))
        return cipher.iv + cipher.doFinal(secret)
    }

    private fun open(alias: String, sealed: ByteArray): ByteArray {
        require(sealed.size > IV_BYTES) { "sealed key is too short" }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(
            Cipher.DECRYPT_MODE,
            key(alias, create = false),
            GCMParameterSpec(TAG_BITS, sealed, 0, IV_BYTES),
        )
        return cipher.doFinal(sealed, IV_BYTES, sealed.size - IV_BYTES)
    }

    companion object {
        const val NOTIFY_CHANNEL = "zelamar/notify"
        const val PUSH_CHANNEL = "zelamar/push"
        const val CLIPBOARD_CHANNEL = "zelamar/clipboard"
        const val KEYSTORE_CHANNEL = "zelamar/keystore"
        const val MESSAGES = "messages"
        const val CALLS = "calls"
        const val ONGOING = "ongoing"
        const val MESSAGE_NOTIFICATION = 1
        const val CALL_NOTIFICATION = 2
        const val ONGOING_NOTIFICATION = 3
        const val PERMISSION_REQUEST = 7
        const val BRONZE = 0xFF8A5A30.toInt()
        const val RING_MS = 60_000L
        const val KEYSTORE = "AndroidKeyStore"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
        const val TAG_BITS = 128
    }
}
