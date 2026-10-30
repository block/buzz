package xyz.block.buzz.mobile

import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/** Native sealing bridge; never returns plaintext or provider error details. */
internal class HostedHpkePlugin(messenger: BinaryMessenger) : MethodChannel.MethodCallHandler {
    private val channel = MethodChannel(
        messenger, "buzz/hosted_hpke", io.flutter.plugin.common.StandardMethodCodec.INSTANCE,
        messenger.makeBackgroundTaskQueue(),
    ).apply { setMethodCallHandler(this@HostedHpkePlugin) }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        if (call.method != "seal") {
            result.notImplemented()
            return
        }
        val arguments = call.arguments as? Map<*, *>
        val plaintext = arguments?.get("plaintext") as? ByteArray
        try {
            val recipient = arguments?.get("recipient") as? ByteArray
            val aad = arguments?.get("aad") as? ByteArray
            if (recipient == null || plaintext == null || aad == null) {
                result.error("invalid_input", null, null)
                return
            }
            result.success(HostedHpkeBackup.seal(recipient, plaintext, aad))
        } catch (_: Exception) {
            result.error("backup_unavailable", null, null)
        } finally {
            plaintext?.fill(0)
        }
    }

    fun dispose() { channel.setMethodCallHandler(null) }
}
