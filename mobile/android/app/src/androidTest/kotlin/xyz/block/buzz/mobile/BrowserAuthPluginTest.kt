package xyz.block.buzz.mobile

import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.FlutterException
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.StandardMethodCodec
import java.nio.ByteBuffer
import org.junit.Test
import org.junit.runner.RunWith
import kotlin.test.assertEquals
import kotlin.test.assertNull

/** Real Android Uri/Intent and registered channel; only browser launch is replaced. */
@RunWith(AndroidJUnit4::class)
class BrowserAuthPluginTest {
    @Test fun closeRetryCallbackAndLateResume() {
        InstrumentationRegistry.getInstrumentation().runOnMainSync { runJourney() }
    }

    private fun runJourney() {
        val messenger = Messenger()
        var launches = 0
        val plugin = BrowserAuthPlugin(Activity(), messenger) { launches++ }
        val first = messenger.start("first")
        plugin.onResume() // Initial resume is not a close.
        assertEquals(0, first.replies)
        plugin.onPause()
        plugin.onResume()
        assertEquals("cancelled", first.error)
        assertEquals(1, first.replies)

        val second = messenger.start("second")
        plugin.onPause()
        plugin.onNewIntent(Intent(Intent.ACTION_VIEW, Uri.parse(
            "xyz.block.buzz.mobile:/auth/callback?state=first&code=old",
        )))
        plugin.onResume() // Return caused by the stale callback, not dismissal.
        assertEquals(0, second.replies)
        plugin.onPause() // The current browser can still complete.
        val callback = "xyz.block.buzz.mobile:/auth/callback?state=second&code=code"
        plugin.onNewIntent(Intent(Intent.ACTION_VIEW, Uri.parse(callback)))
        plugin.onResume()
        assertEquals(callback, second.value)
        assertNull(second.error)
        assertEquals(1, second.replies)
        assertEquals(2, launches)

        val third = messenger.start("third")
        messenger.call("cancel", null)
        plugin.onPause()
        plugin.onResume()
        assertEquals("cancelled", third.error)
        assertEquals(1, third.replies)
        val fourth = messenger.start("fourth")
        plugin.onPause()
        plugin.onNewIntent(Intent(Intent.ACTION_VIEW, Uri.parse(callback)))
        plugin.onResume()
        assertEquals(0, fourth.replies)
        plugin.onPause() // Returning to the browser re-arms dismissal detection.
        plugin.onResume()
        assertEquals("cancelled", fourth.error)
        assertEquals(1, fourth.replies)
        plugin.dispose()
        assertNull(messenger.handler)
    }

    private class Reply {
        var replies = 0
        var value: Any? = null
        var error: String? = null
    }

    private class Messenger : BinaryMessenger {
        var handler: BinaryMessenger.BinaryMessageHandler? = null
        fun start(state: String) = call("start", mapOf(
            "url" to "https://app.builderlab.xyz/api/goose/v1/auth/login?state=$state",
        ))
        fun call(method: String, arguments: Any?): Reply {
            val reply = Reply()
            val message = StandardMethodCodec.INSTANCE.encodeMethodCall(MethodCall(method, arguments))
            message.flip()
            handler!!.onMessage(message) { response ->
                reply.replies++
                response!!.flip()
                try {
                    reply.value = StandardMethodCodec.INSTANCE.decodeEnvelope(response)
                } catch (error: FlutterException) {
                    reply.error = error.code
                }
            }
            return reply
        }
        override fun send(channel: String, message: ByteBuffer?) = Unit
        override fun send(channel: String, message: ByteBuffer?, callback: BinaryMessenger.BinaryReply?) = Unit
        override fun setMessageHandler(channel: String, handler: BinaryMessenger.BinaryMessageHandler?) {
            assertEquals("buzz/browser_auth", channel)
            this.handler = handler
        }
    }
}
