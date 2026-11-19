package xyz.block.buzz.mobile

import java.math.BigInteger
import java.security.KeyFactory
import java.security.KeyPair
import java.security.spec.ECPrivateKeySpec
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse

class HostedHpkeBackupTest {
    // RFC 9180 reference vectors: cfrg/draft-irtf-cfrg-hpke/test-vectors.json,
    // mode=0, kem_id=16, kdf_id=1, aead_id=2, sequence number zero.
    @Test fun `RFC 9180 base mode known answer`() {
        val publicKey = HostedHpkeBackup.decodePublicKey(hex(
            "04c06b4f6bebc7bb495cb797ab753f911aff80aefb86fd8b6fcc35525f3ab5f03e0b" +
                "21bd31a86c6048af3cb2d98e0d3bf01da5cc4c39ff5370d331a4f1f7d5a4e0",
        ))
        val privateKey = KeyFactory.getInstance("EC").generatePrivate(ECPrivateKeySpec(
            BigInteger("90345e3a1d116c1dd39ae76d95ab858c142223a63e44f8f85318cfa91a84858e", 16),
            publicKey.params,
        ))
        val recipient = hex(
            "04abc7e49a4c6b3566d77d0304addc6ed0e98512ffccf505e6a8e3eb25c685136f8" +
                "53148544876de76c0f2ef99cdc3a05ccf5ded7860c7c021238f9e2073d2356c",
        )
        val result = HostedHpkeBackup.sealBase(
            recipient, hex("4265617574792069732074727574682c20747275746820626561757479"),
            hex("436f756e742d30"), hex("4f6465206f6e2061204772656369616e2055726e"),
            KeyPair(publicKey, privateKey),
        )
        assertContentEquals(hex(
            "58c61a45059d0c5704560e9d88b564a8b63f1364b8d1fcb3c4c6ddc1d291742465e902cd216f8908da49f8f96f",
        ), result["ciphertext"])
    }

    @Test fun `backup sender validates input and generates fresh encapsulation`() {
        val first = HostedHpkeBackup.seal(recipient, ByteArray(32) { 0x42 }, byteArrayOf(1))
        val second = HostedHpkeBackup.seal(recipient, ByteArray(32) { 0x42 }, byteArrayOf(1))
        assertEquals(65, first.getValue("enc").size)
        assertEquals(48, first.getValue("ciphertext").size)
        assertFalse(first.getValue("enc").contentEquals(second.getValue("enc")))
        for (bad in listOf(ByteArray(65), recipient.copyOf(33), recipient.copyOf().apply { this[64] = 0 })) {
            assertFailsWith<IllegalArgumentException> { HostedHpkeBackup.seal(bad, ByteArray(32), byteArrayOf(1)) }
        }
        assertFailsWith<IllegalArgumentException> { HostedHpkeBackup.seal(recipient, ByteArray(31), byteArrayOf(1)) }
        assertFailsWith<IllegalArgumentException> { HostedHpkeBackup.seal(recipient, ByteArray(32), ByteArray(0)) }
    }

    companion object {
        fun hex(value: String) = value.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
        val recipient = hex(
            "04c5b6519400186a8e10abbdc05d3e450c8612e5dba06927091527110324acfb0371" +
                "d98d662395d872fae967fe75ef24db20a7c2c1af19c34422d94f629b1b1dad",
        )
    }
}
