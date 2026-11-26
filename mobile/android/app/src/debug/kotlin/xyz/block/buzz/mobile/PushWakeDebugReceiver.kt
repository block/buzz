package xyz.block.buzz.mobile

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import kotlin.concurrent.thread

/**
 * Debug builds only: delivers a push wake the way FCM does, for testing the
 * handler without a gateway.
 *
 *     adb shell am broadcast -n <applicationId>/xyz.block.buzz.mobile.PushWakeDebugReceiver
 */
class PushWakeDebugReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val pending = goAsync()
        thread {
            try {
                PushWake.handle(context)
            } finally {
                pending.finish()
            }
        }
    }
}
