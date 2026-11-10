package xyz.block.buzz.mobile

import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.StandardMethodCodec
import java.nio.ByteBuffer
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

class HostedHpkePluginTest {
    @Test fun `registered channel seals and clears native plaintext`() {
        val messenger = Messenger()
        val plugin = HostedHpkePlugin(messenger)
        assertEquals("buzz/hosted_hpke", messenger.channel)
        assertNotNull(messenger.queue)
        val secret = ByteArray(32) { 0x42 }
        val reply = Reply()
        plugin.onMethodCall(MethodCall("seal", mapOf(
            "recipient" to HostedHpkeBackupTest.recipient, "plaintext" to secret, "aad" to byteArrayOf(1),
        )), reply)
        assertNull(reply.code)
        val envelope = reply.value as Map<*, *>
        assertEquals(65, (envelope["enc"] as ByteArray).size)
        assertEquals(48, (envelope["ciphertext"] as ByteArray).size)
        assertContentEquals(ByteArray(32), secret)
        // Traverse the actual registered codec handler for a successful seal.
        val encoded = StandardMethodCodec.INSTANCE.encodeMethodCall(MethodCall("seal", mapOf(
            "recipient" to HostedHpkeBackupTest.recipient,
            "plaintext" to ByteArray(32) { 0x42 }, "aad" to byteArrayOf(1),
        )))
        encoded.flip()
        var dispatched = false
        messenger.handler!!.onMessage(encoded) {
            it!!.flip()
            val sealed = StandardMethodCodec.INSTANCE.decodeEnvelope(it) as Map<*, *>
            assertEquals(48, (sealed["ciphertext"] as ByteArray).size)
            dispatched = true
        }
        assertEquals(true, dispatched)
        val call = StandardMethodCodec.INSTANCE.encodeMethodCall(MethodCall("unknown", null))
        call.flip()
        messenger.handler!!.onMessage(call) { assertNull(it) }
        plugin.dispose()
        assertNull(messenger.handler)
    }

    @Test fun `invalid input is redacted and plaintext cleared on failure`() {
        val plugin = HostedHpkePlugin(Messenger())
        for (recipient in listOf(null, ByteArray(65))) {
            val secret = ByteArray(32) { 0x42 }
            val reply = Reply()
            plugin.onMethodCall(MethodCall("seal", mapOf(
                "recipient" to recipient, "plaintext" to secret, "aad" to byteArrayOf(1),
            )), reply)
            assertEquals(if (recipient == null) "invalid_input" else "backup_unavailable", reply.code)
            assertNull(reply.value)
            assertContentEquals(ByteArray(32), secret)
        }
    }

    private class Reply : MethodChannel.Result {
        var value: Any? = null
        var code: String? = null
        override fun success(result: Any?) { value = result }
        override fun error(errorCode: String, errorMessage: String?, errorDetails: Any?) {
            code = errorCode
            assertNull(errorMessage)
            assertNull(errorDetails)
        }
        override fun notImplemented() { error("Unexpected missing method") }
    }

    private class Messenger : BinaryMessenger {
        var channel: String? = null
        var handler: BinaryMessenger.BinaryMessageHandler? = null
        var queue: BinaryMessenger.TaskQueue? = null
        override fun makeBackgroundTaskQueue(options: BinaryMessenger.TaskQueueOptions) =
            object : BinaryMessenger.TaskQueue {}
        override fun send(channel: String, message: ByteBuffer?) = Unit
        override fun send(channel: String, message: ByteBuffer?, callback: BinaryMessenger.BinaryReply?) = Unit
        override fun setMessageHandler(channel: String, handler: BinaryMessenger.BinaryMessageHandler?) {
            this.channel = channel
            this.handler = handler
        }
        override fun setMessageHandler(channel: String, handler: BinaryMessenger.BinaryMessageHandler?, taskQueue: BinaryMessenger.TaskQueue?) {
            this.queue = taskQueue
            setMessageHandler(channel, handler)
        }
    }
}
