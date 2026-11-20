package xyz.block.buzz.mobile

import android.content.Intent
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel

/**
 * Hands the app a tapped push notification, through the channel iOS uses, so
 * it opens the conversation by the same deep-link handling.
 *
 * A tap that starts the app is held until Dart asks for it, since its handler
 * isn't installed yet. A tap while the app runs is delivered at once.
 */
internal class PushNotificationOpenPlugin(messenger: BinaryMessenger) {
    private val channel = MethodChannel(messenger, "buzz/push")
    private var pending: Map<String, String>? = null

    init {
        channel.setMethodCallHandler { call, result ->
            when (call.method) {
                "takePendingNotificationResponse" -> {
                    result.success(pending)
                    pending = null
                }
                else -> result.notImplemented()
            }
        }
    }

    /** The activity was started by [intent], which may be a notification tap. */
    fun onLaunch(intent: Intent?) {
        // Returning from recents replays the intent that first started the
        // activity, and that tap was already followed.
        if (intent != null && intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0) return
        pending = NotificationOpen.arguments(intent) ?: pending
    }

    /** The running activity was brought forward by [intent]. */
    fun onNewIntent(intent: Intent) {
        val arguments = NotificationOpen.arguments(intent) ?: return
        channel.invokeMethod(
            "notificationOpened",
            arguments,
            object : MethodChannel.Result {
                override fun success(result: Any?) = Unit
                override fun error(code: String, message: String?, details: Any?) {
                    pending = arguments
                }
                override fun notImplemented() {
                    pending = arguments
                }
            },
        )
    }

    fun dispose() {
        channel.setMethodCallHandler(null)
    }
}
