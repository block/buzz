package xyz.block.buzz.mobile

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import java.security.InvalidAlgorithmParameterException
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.ProviderException
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec

/**
 * The push installation's Keystore key: a P-256 key that can only sign, with a
 * key attestation chain whose challenge Dart supplies. The private key never
 * leaves the Keystore. Dart owns every decision; this only generates, signs
 * and reports.
 */
internal class PushKeystorePlugin(messenger: BinaryMessenger) : MethodChannel.MethodCallHandler {
    private val channel = MethodChannel(
        messenger, "buzz/push_keystore", io.flutter.plugin.common.StandardMethodCodec.INSTANCE,
        messenger.makeBackgroundTaskQueue(),
    ).apply { setMethodCallHandler(this@PushKeystorePlugin) }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        try {
            when (call.method) {
                "current" -> result.success(describe())
                "generate" -> {
                    val challenge = (call.arguments as? Map<*, *>)?.get("challenge") as? ByteArray
                    if (challenge == null) {
                        result.error("invalid_arguments", "Expected an attestation challenge.", null)
                    } else {
                        generate(challenge, result)
                    }
                }
                "sign" -> {
                    val data = (call.arguments as? Map<*, *>)?.get("data") as? ByteArray
                    if (data == null) {
                        result.error("invalid_arguments", "Expected bytes to sign.", null)
                    } else {
                        result.success(sign(data))
                    }
                }
                "delete" -> {
                    keyStore().deleteEntry(ALIAS)
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        } catch (error: Exception) {
            result.error("keystore_failed", error.message, error.javaClass.simpleName)
        }
    }

    private fun keyStore(): KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun generate(challenge: ByteArray, result: MethodChannel.Result) {
        val store = keyStore()
        store.deleteEntry(ALIAS)
        try {
            KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore").apply {
                initialize(
                    KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_SIGN)
                        .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
                        .setDigests(KeyProperties.DIGEST_SHA256)
                        .setAttestationChallenge(challenge)
                        .build(),
                )
            }.generateKeyPair()
        } catch (error: ProviderException) {
            result.error("attestation_unavailable", error.message ?: "key generation failed", null)
            return
        } catch (error: InvalidAlgorithmParameterException) {
            result.error("attestation_unavailable", error.message ?: "key generation failed", null)
            return
        }
        // A phone without hardware attestation hands back a lone self-signed
        // certificate. That chain proves nothing, so it never leaves here.
        if ((store.getCertificateChain(ALIAS)?.size ?: 0) < 2) {
            store.deleteEntry(ALIAS)
            result.error("attestation_unavailable", "no attestation chain", null)
            return
        }
        result.success(describe())
    }

    private fun describe(): Map<String, Any>? {
        val store = keyStore()
        if (!store.containsAlias(ALIAS)) return null
        val chain = store.getCertificateChain(ALIAS) ?: return null
        val point = (chain[0].publicKey as ECPublicKey).w
        return mapOf(
            "publicKey" to byteArrayOf(0x04) + fixed32(point.affineX.toByteArray()) + fixed32(point.affineY.toByteArray()),
            "chain" to chain.map { it.encoded },
        )
    }

    private fun sign(data: ByteArray): ByteArray {
        val key = keyStore().getKey(ALIAS, null) as? PrivateKey
            ?: throw IllegalStateException("The push installation key is missing.")
        return Signature.getInstance("SHA256withECDSA").run {
            initSign(key)
            update(data)
            sign()
        }
    }

    /** A big-endian integer as exactly 32 bytes: BigInteger adds or drops a sign byte. */
    private fun fixed32(bytes: ByteArray): ByteArray {
        val trimmed = if (bytes.size > 32) bytes.copyOfRange(bytes.size - 32, bytes.size) else bytes
        return ByteArray(32 - trimmed.size) + trimmed
    }

    fun dispose() { channel.setMethodCallHandler(null) }

    private companion object {
        const val ALIAS = "xyz.block.buzz.push.installation.v1"
    }
}
