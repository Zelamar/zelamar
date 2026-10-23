package com.zelamar.app

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.PersistableBundle
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Hosts the app; shows notifications for new messages ("zelamar/notify",
 * lib/src/messenger/notifier.dart); hands out the Firebase token for push
 * wake-ups ("zelamar/push", ADR-0017); and seals Zelamar's master key with the
 * Android Keystore
 * (lib/src/messenger/os_vault.dart): a hardware-backed AES-256-GCM key that
 * never leaves the device. Sealed bytes are `IV (12) || ciphertext || tag`.
 */
class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, CHANNEL)
            .setMethodCallHandler { call, result ->
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
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, PUSH_CHANNEL)
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "token" -> pushToken { result.success(it) }
                    else -> result.notImplemented()
                }
            }
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, CLIPBOARD_CHANNEL)
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "copySecret" -> result.success(copySecret(call.arguments as? String))
                    else -> result.notImplemented()
                }
            }
        createMessageChannel()
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, NOTIFY_CHANNEL)
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "show" -> {
                        notify(
                            call.argument<String>("title").orEmpty(),
                            call.argument<String>("body").orEmpty(),
                        )
                        result.success(null)
                    }
                    "inFront" -> result.success(hasWindowFocus())
                    "requestPermission" -> {
                        requestNotificationPermission()
                        result.success(null)
                    }
                    else -> result.notImplemented()
                }
            }
    }

    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), PERMISSION_REQUEST)
        }
    }

    /**
     * This device's Firebase token, or `null` when the build has no Firebase
     * app or the token cannot be had right now.
     */
    private fun pushToken(done: (String?) -> Unit) {
        if (FirebaseApp.getApps(this).isEmpty()) {
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

    /** The channel for messages, which push wake-ups use too. */
    private fun createMessageChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            manager.createNotificationChannel(
                NotificationChannel(
                    MESSAGES,
                    "Messages",
                    NotificationManager.IMPORTANCE_HIGH,
                ).apply { description = "New messages" },
            )
        }
    }

    /** One notification per sender would be next; for now, the latest message. */
    private fun notify(title: String, body: String) {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        createMessageChannel()
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, MESSAGES)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this).setPriority(Notification.PRIORITY_HIGH)
        }
        val notification = builder
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
        manager.notify(MESSAGE_NOTIFICATION, notification)
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
        (getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(clip)
        return true
    }

    /** What a locked screen shows instead of a message. */
    private fun publicNotification(open: PendingIntent): Notification {
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, MESSAGES)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
        return builder
            .setSmallIcon(R.drawable.ic_stat_zelamar)
            .setColor(BRONZE)
            .setContentTitle("Zelamar")
            .setContentText("New message")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
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

    private companion object {
        const val NOTIFY_CHANNEL = "zelamar/notify"
        const val PUSH_CHANNEL = "zelamar/push"
        const val CLIPBOARD_CHANNEL = "zelamar/clipboard"
        const val MESSAGES = "messages"
        const val MESSAGE_NOTIFICATION = 1
        const val PERMISSION_REQUEST = 7
        const val BRONZE = 0xFF8A5A30.toInt()
        const val CHANNEL = "zelamar/keystore"
        const val KEYSTORE = "AndroidKeyStore"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
        const val TAG_BITS = 128
    }
}
