package com.zelamar.app

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Hosts the app and seals Zelamar's master key with the Android Keystore
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
        const val CHANNEL = "zelamar/keystore"
        const val KEYSTORE = "AndroidKeyStore"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
        const val TAG_BITS = 128
    }
}
