package xyz.block.buzz.mobile

import java.math.BigInteger
import java.security.AlgorithmParameters
import java.security.KeyFactory
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.ECParameterSpec
import java.security.spec.ECPoint
import java.security.spec.ECPublicKeySpec
import javax.crypto.Cipher
import javax.crypto.KeyAgreement
import javax.crypto.Mac
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/** RFC 9180 base mode, DHKEM(P-256, HKDF-SHA256), HKDF-SHA256, AES-256-GCM.
 * Enrollment and AAD must come from the authenticated hosted coordinator.
 * Only JCA platform primitives are used; this is a single-message sender.
 */
internal object HostedHpkeBackup {
    private val kemSuite = "KEM".toByteArray() + byteArrayOf(0, 16)
    private val suite = "HPKE".toByteArray() + byteArrayOf(0, 16, 0, 1, 0, 2)
    private val version = "HPKE-v1".toByteArray()
    private val info = "buzz/nsec-backup/v1".toByteArray()
    private val curve: ECParameterSpec
        get() = AlgorithmParameters.getInstance("EC").apply {
            init(ECGenParameterSpec("secp256r1"))
        }.getParameterSpec(ECParameterSpec::class.java)

    fun seal(recipient: ByteArray, plaintext: ByteArray, aad: ByteArray): Map<String, ByteArray> {
        require(plaintext.size == 32 && aad.isNotEmpty())
        val ephemeral = KeyPairGenerator.getInstance("EC").apply {
            initialize(ECGenParameterSpec("secp256r1"))
        }.generateKeyPair()
        return sealBase(recipient, plaintext, aad, info, ephemeral)
    }

    // Internal seam for the RFC known-answer vector. The channel only calls seal,
    // which fixes the protocol info and always generates a fresh ephemeral key.
    internal fun sealBase(
        recipient: ByteArray,
        plaintext: ByteArray,
        aad: ByteArray,
        info: ByteArray,
        ephemeral: KeyPair,
    ): Map<String, ByteArray> {
        val publicKey = decodePublicKey(recipient)
        val enc = encodePublicKey(ephemeral.public as ECPublicKey)
        val dh = KeyAgreement.getInstance("ECDH").run {
            init(ephemeral.private)
            doPhase(publicKey, true)
            generateSecret()
        }
        var shared = ByteArray(0)
        var secret = ByteArray(0)
        var key = ByteArray(0)
        try {
            val eae = extract(kemSuite, ByteArray(0), "eae_prk", dh)
            try {
                shared = expand(kemSuite, eae, "shared_secret", enc + recipient, 32)
            } finally {
                eae.fill(0)
            }
            val context = byteArrayOf(0) +
                extract(suite, ByteArray(0), "psk_id_hash", ByteArray(0)) +
                extract(suite, ByteArray(0), "info_hash", info)
            secret = extract(suite, shared, "secret", ByteArray(0))
            key = expand(suite, secret, "key", context, 32)
            val nonce = expand(suite, secret, "base_nonce", context, 12)
            // Sequence number is zero: the first and only seal in this context.
            val ciphertext = Cipher.getInstance("AES/GCM/NoPadding").run {
                init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(128, nonce))
                updateAAD(aad)
                doFinal(plaintext)
            }
            return mapOf("enc" to enc, "ciphertext" to ciphertext)
        } finally {
            dh.fill(0)
            shared.fill(0)
            secret.fill(0)
            key.fill(0)
        }
    }

    internal fun decodePublicKey(encoded: ByteArray): ECPublicKey {
        require(encoded.size == 65 && encoded[0] == 4.toByte())
        val parameters = curve
        val x = BigInteger(1, encoded.copyOfRange(1, 33))
        val y = BigInteger(1, encoded.copyOfRange(33, 65))
        val p = (parameters.curve.field as java.security.spec.ECFieldFp).p
        // Validate the SEC1 point ourselves: provider validation differs by API.
        require(x < p && y < p)
        require(y.multiply(y).mod(p) ==
            x.multiply(x).multiply(x).add(parameters.curve.a.multiply(x))
                .add(parameters.curve.b).mod(p))
        return KeyFactory.getInstance("EC").generatePublic(
            ECPublicKeySpec(ECPoint(x, y), parameters),
        ) as ECPublicKey
    }

    private fun encodePublicKey(key: ECPublicKey): ByteArray {
        fun coordinate(value: BigInteger): ByteArray {
            val bytes = value.toByteArray()
            return ByteArray(32).also {
                val length = minOf(bytes.size, 32)
                bytes.copyInto(it, 32 - length, bytes.size - length)
            }
        }
        return byteArrayOf(4) + coordinate(key.w.affineX) + coordinate(key.w.affineY)
    }

    private fun hmac(key: ByteArray, data: ByteArray): ByteArray =
        Mac.getInstance("HmacSHA256").run {
            init(SecretKeySpec(if (key.isEmpty()) ByteArray(32) else key, "HmacSHA256"))
            doFinal(data)
        }

    private fun extract(suite: ByteArray, salt: ByteArray, label: String, input: ByteArray): ByteArray {
        val labeledInput = version + suite + label.toByteArray() + input
        return try { hmac(salt, labeledInput) } finally { labeledInput.fill(0) }
    }

    private fun expand(
        suite: ByteArray, prk: ByteArray, label: String, context: ByteArray, length: Int,
    ): ByteArray {
        // Every output in this fixed suite is at most one SHA-256 block.
        require(length in 1..32)
        val labeledInfo = byteArrayOf(0, length.toByte()) + version + suite +
            label.toByteArray() + context
        val block = hmac(prk, labeledInfo + byteArrayOf(1))
        return try { block.copyOf(length) } finally { block.fill(0) }
    }
}
