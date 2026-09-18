package xyz.block.buzz.mobile

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.google.android.gms.tasks.Tasks
import com.google.firebase.installations.FirebaseInstallations
import com.google.firebase.messaging.FirebaseMessaging
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import java.lang.ref.WeakReference
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

internal class BuzzAndroidPushPlugin(
    private val activity: Activity,
    messenger: BinaryMessenger,
) {
    private val channel = MethodChannel(messenger, CHANNEL)
    private val executor = Executors.newSingleThreadExecutor()
    private val store = BuzzAndroidPushStore(activity.applicationContext)
    private val enrollment = BuzzAndroidPushEnrollment(activity.applicationContext)

    init {
        activeChannel = WeakReference(channel)
        BuzzFirebaseMessagingService.createChannel(activity)
        channel.setMethodCallHandler(::handle)
        store.fcmEndpoint()?.let(::notifyEndpointChanged)
    }

    fun dispose() {
        channel.setMethodCallHandler(null)
        if (activeChannel?.get() === channel) activeChannel = null
        executor.shutdownNow()
    }

    fun onRequestPermissionsResult(requestCode: Int, grantResults: IntArray): Boolean {
        if (requestCode != NOTIFICATION_PERMISSION_REQUEST) return false
        activity.getSharedPreferences(PREFERENCES, Activity.MODE_PRIVATE)
            .edit()
            .putBoolean(PERMISSION_REQUESTED, true)
            .apply()
        return true
    }

    fun notificationOpened(intent: Intent?) {
        if (intent?.getBooleanExtra(BuzzFirebaseMessagingService.OPENED_EXTRA, false) != true) return
        intent.removeExtra(BuzzFirebaseMessagingService.OPENED_EXTRA)
        channel.invokeMethod("notificationOpened", null)
    }

    private fun handle(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "startRegistration" -> startRegistration(result)
            "notificationAuthorizationStatus" -> result.success(authorizationStatus())
            "openNotificationSettings" -> result.success(openNotificationSettings())
            "takePendingNotificationResponse" -> result.success(null)
            "endpointGrants" -> result.success(
                enrollment.endpointGrants().map(BuzzAndroidPushGrant::flutterArguments),
            )
            "enrollPush" -> enroll(call, result)
            else -> result.notImplemented()
        }
    }

    private fun startRegistration(result: MethodChannel.Result) {
        if (!BuzzFirebaseConfiguration.initialize(activity.application)) {
            result.error(
                "firebase_not_configured",
                "Android notifications require the Firebase build configuration.",
                null,
            )
            return
        }
        requestNotificationPermissionIfNeeded()
        executor.execute {
            try {
                val endpoint = registerFcmEndpoint()
                activity.runOnUiThread {
                    notifyEndpointChanged(endpoint)
                    result.success(null)
                }
            } catch (error: Exception) {
                activity.runOnUiThread {
                    notifyRegistrationFailed(error.message ?: "FCM registration failed")
                    result.error("fcm_registration_failed", "FCM registration failed.", null)
                }
            }
        }
    }

    private fun enroll(call: MethodCall, result: MethodChannel.Result) {
        val relayUrl = call.argument<String>("relayUrl")
        val gatewayUrl = call.argument<String>("gatewayUrl")
        if (relayUrl.isNullOrBlank() || gatewayUrl.isNullOrBlank()) {
            result.error("invalid_arguments", "Push enrollment requires relayUrl and gatewayUrl.", null)
            return
        }
        executor.execute {
            try {
                val endpoint = store.fcmEndpoint() ?: registerFcmEndpoint()
                val grant = enrollment.enroll(endpoint, relayUrl, gatewayUrl)
                activity.runOnUiThread { result.success(grant.flutterArguments()) }
            } catch (error: Exception) {
                activity.runOnUiThread {
                    result.error(
                        "android_enrollment_failed",
                        "Android push enrollment failed.",
                        error.message,
                    )
                }
            }
        }
    }

    private fun registerFcmEndpoint(): String {
        Tasks.await(FirebaseMessaging.getInstance().register(), 20, TimeUnit.SECONDS)
        val endpoint = Tasks.await(FirebaseInstallations.getInstance().id, 10, TimeUnit.SECONDS)
        require(endpoint.isNotBlank()) { "FCM registration returned an empty installation ID" }
        store.saveFcmEndpoint(endpoint)
        return endpoint
    }

    private fun requestNotificationPermissionIfNeeded() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        if (ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        ) return
        ActivityCompat.requestPermissions(
            activity,
            arrayOf(Manifest.permission.POST_NOTIFICATIONS),
            NOTIFICATION_PERMISSION_REQUEST,
        )
    }

    private fun authorizationStatus(): String {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            val requested = activity.getSharedPreferences(PREFERENCES, Activity.MODE_PRIVATE)
                .getBoolean(PERMISSION_REQUESTED, false)
            return if (requested) "denied" else "notDetermined"
        }
        return if (NotificationManagerCompat.from(activity).areNotificationsEnabled()) {
            "authorized"
        } else {
            "denied"
        }
    }

    private fun openNotificationSettings(): Boolean = runCatching {
        activity.startActivity(
            Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).apply {
                data = Uri.parse("package:${activity.packageName}")
                putExtra(Settings.EXTRA_APP_PACKAGE, activity.packageName)
            },
        )
        true
    }.getOrDefault(false)

    companion object {
        private const val CHANNEL = "buzz/push"
        private const val NOTIFICATION_PERMISSION_REQUEST = 0x4255
        private const val PREFERENCES = "buzz_android_push_permission"
        private const val PERMISSION_REQUESTED = "requested"
        private var activeChannel: WeakReference<MethodChannel>? = null

        fun notifyEndpointChanged(endpoint: String) {
            Handler(Looper.getMainLooper()).post {
                activeChannel?.get()?.invokeMethod(
                    "fcmTokenChanged",
                    mapOf("token" to endpoint),
                )
            }
        }

        fun notifyRegistrationFailed(message: String) {
            Handler(Looper.getMainLooper()).post {
                activeChannel?.get()?.invokeMethod(
                    "fcmRegistrationFailed",
                    mapOf("message" to message),
                )
            }
        }
    }
}
