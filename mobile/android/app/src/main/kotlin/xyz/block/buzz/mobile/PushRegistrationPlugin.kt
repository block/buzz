package xyz.block.buzz.mobile

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/**
 * Notification permission and the FCM registration token, on a channel of
 * its own: `buzz/push` has one native handler, which opens tapped
 * notifications. The token and a failure to get one reach Dart as calls, like
 * the iOS APNs callbacks.
 *
 * The permission prompt never gates the token: the installation is enrolled
 * whether or not notifications may be displayed, as on iOS.
 */
internal class PushRegistrationPlugin(
    private val activity: Activity,
    messenger: BinaryMessenger,
) : MethodChannel.MethodCallHandler {
    private val channel = MethodChannel(messenger, "buzz/push_registration").apply {
        setMethodCallHandler(this@PushRegistrationPlugin)
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "startRegistration" -> {
                // A build without Firebase has no token to wait for, and must
                // not ask the user for a permission that only push needs.
                if (firebaseConfigured()) {
                    requestNotificationPermission()
                    fetchToken()
                }
                result.success(null)
            }
            "refreshToken" -> {
                if (firebaseConfigured()) fetchToken()
                result.success(null)
            }
            else -> result.notImplemented()
        }
    }

    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val permission = Manifest.permission.POST_NOTIFICATIONS
        if (ContextCompat.checkSelfPermission(activity, permission) == PackageManager.PERMISSION_GRANTED) return
        // Ask once, as iOS does. A decline stands until the user changes it in
        // system settings; asking again on every launch would nag.
        val asked = activity.getSharedPreferences(PREFERENCES, android.content.Context.MODE_PRIVATE)
        if (asked.getBoolean(ASKED_KEY, false)) return
        asked.edit().putBoolean(ASKED_KEY, true).apply()
        ActivityCompat.requestPermissions(activity, arrayOf(permission), REQUEST_CODE)
    }

    /** Whether Firebase is set up; when not, tells Dart registration failed. */
    private fun firebaseConfigured(): Boolean {
        if (FirebaseApp.getApps(activity).isNotEmpty()) return true
        channel.invokeMethod(
            "fcmRegistrationFailed",
            mapOf("message" to "Firebase is not configured in this build."),
        )
        return false
    }

    private fun fetchToken() {
        FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
            // `result` throws for a failed task, so read it only after success.
            val token = if (task.isSuccessful) task.result else null
            if (!token.isNullOrEmpty()) {
                channel.invokeMethod("fcmTokenChanged", mapOf("token" to token))
            } else {
                channel.invokeMethod(
                    "fcmRegistrationFailed",
                    mapOf("message" to (task.exception?.message ?: "FCM returned no token.")),
                )
            }
        }
    }

    fun dispose() { channel.setMethodCallHandler(null) }

    companion object {
        const val REQUEST_CODE = 0x6275
        private const val PREFERENCES = "buzz_push"
        private const val ASKED_KEY = "notification_permission_asked"
    }
}
