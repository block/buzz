package xyz.block.buzz.mobile

import android.app.Activity
import android.os.Build
import android.view.HapticFeedbackConstants
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel

/** Platform error feedback respects the user's system haptic preference. */
internal class HapticsPlugin(activity: Activity, messenger: BinaryMessenger) {
    private val channel = MethodChannel(messenger, "buzz/haptics")

    init {
        channel.setMethodCallHandler { call, result ->
            if (call.method == "error") {
                val feedback = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    HapticFeedbackConstants.REJECT
                } else {
                    HapticFeedbackConstants.LONG_PRESS
                }
                activity.window.decorView.performHapticFeedback(feedback)
                result.success(null)
            } else {
                result.notImplemented()
            }
        }
    }

    fun dispose() { channel.setMethodCallHandler(null) }
}
