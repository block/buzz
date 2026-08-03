package xyz.block.buzz.mobile

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import xyz.block.buzz.mobile.AndroidNotificationContract.NOTIFICATION_ROUTE_EXTRA
import xyz.block.buzz.mobile.AndroidNotificationContract.NOTIFICATION_TAP_ACTION

internal class AndroidNotificationBridge(
    private val activity: MainActivity,
    binaryMessenger: BinaryMessenger,
) {
    private val notificationManager =
        activity.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    private val channel = MethodChannel(binaryMessenger, METHOD_CHANNEL)
    private var initialRoute = consumeNotificationRoute(activity.intent)
    private var pendingPermissionResult: MethodChannel.Result? = null

    init {
        channel.setMethodCallHandler(::handleMethodCall)
    }

    fun handleIntent(intent: Intent) {
        activity.setIntent(intent)
        val route = consumeNotificationRoute(intent) ?: return
        channel.invokeMethod(NOTIFICATION_TAPPED_METHOD, route)
    }

    fun handleResume() {
        channel.invokeMethod(STATUS_CHANGED_METHOD, status())
    }

    fun handlePermissionResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        if (requestCode != NOTIFICATION_PERMISSION_REQUEST_CODE) return
        val result = pendingPermissionResult ?: return
        pendingPermissionResult = null
        if (!AndroidNotificationContract.isValidPermissionCallback(
                permissions = permissions,
                grantResultCount = grantResults.size,
                notificationPermission = Manifest.permission.POST_NOTIFICATIONS,
            )
        ) {
            result.error(
                "invalid_permission_result",
                "Android returned an invalid notification permission result.",
                null,
            )
            return
        }
        result.success(status())
    }

    fun dispose() {
        channel.setMethodCallHandler(null)
        pendingPermissionResult?.error(
            "activity_unavailable",
            "The notification permission request was interrupted.",
            null,
        )
        pendingPermissionResult = null
    }

    private fun handleMethodCall(
        call: MethodCall,
        result: MethodChannel.Result,
    ) {
        when (call.method) {
            GET_STATUS_METHOD -> withNoArguments(call, result) { result.success(status()) }
            REQUEST_PERMISSION_METHOD -> withNoArguments(call, result) {
                requestPermission(result)
            }
            ENSURE_CHANNELS_METHOD -> withNoArguments(call, result) {
                ensureChannels()
                result.success(null)
            }
            SHOW_METHOD -> show(call.arguments, result)
            OPEN_SETTINGS_METHOD -> withNoArguments(call, result) { openSettings(result) }
            GET_INITIAL_ROUTE_METHOD -> withNoArguments(call, result) {
                val route = initialRoute
                initialRoute = null
                result.success(route)
            }
            else -> result.notImplemented()
        }
    }

    private fun withNoArguments(
        call: MethodCall,
        result: MethodChannel.Result,
        action: () -> Unit,
    ) {
        if (call.arguments != null) {
            invalidArguments(result, "${call.method} does not accept arguments.")
            return
        }
        action()
    }

    private fun status(): Map<String, Any> {
        // Creating an existing channel refreshes app-owned metadata without
        // overwriting the user's importance choice. It also makes every status
        // read authoritative after a channel was deleted in Android settings.
        ensureChannels()
        return mapOf(
            "permission" to permissionStatus(),
            "priorityChannelEnabled" to channelEnabled(PRIORITY_CHANNEL_ID),
            "activityChannelEnabled" to channelEnabled(ACTIVITY_CHANNEL_ID),
        )
    }

    private fun permissionStatus(): String {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return "notRequired"
        if (activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            return "granted"
        }
        return if (permissionWasRequested()) "denied" else "notDetermined"
    }

    private fun requestPermission(result: MethodChannel.Result) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            result.success(status())
            return
        }
        if (pendingPermissionResult != null) {
            result.error(
                "permission_request_in_progress",
                "A notification permission request is already in progress.",
                null,
            )
            return
        }

        markPermissionRequested()
        pendingPermissionResult = result
        try {
            activity.requestPermissions(
                arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                NOTIFICATION_PERMISSION_REQUEST_CODE,
            )
        } catch (_: RuntimeException) {
            pendingPermissionResult = null
            result.error(
                "permission_request_failed",
                "Android could not start the notification permission request.",
                null,
            )
        }
    }

    private fun ensureChannels() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return

        val notificationAudio = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_NOTIFICATION)
            .build()
        val priorityChannel = NotificationChannel(
            PRIORITY_CHANNEL_ID,
            "Mentions and direct messages",
            NotificationManager.IMPORTANCE_DEFAULT,
        ).apply {
            description = "Mentions and direct messages"
            lockscreenVisibility = Notification.VISIBILITY_PRIVATE
            setSound(
                RingtoneManager.getDefaultUri(RingtoneManager.TYPE_NOTIFICATION),
                notificationAudio,
            )
        }
        val activityChannel = NotificationChannel(
            ACTIVITY_CHANNEL_ID,
            "Channel activity",
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = "Other channel activity"
            lockscreenVisibility = Notification.VISIBILITY_PRIVATE
            enableVibration(false)
            setSound(null, null)
        }
        notificationManager.createNotificationChannels(
            listOf(priorityChannel, activityChannel),
        )
    }

    private fun channelEnabled(channelId: String): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N &&
            !notificationManager.areNotificationsEnabled()
        ) {
            return false
        }
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return true
        val channel = notificationManager.getNotificationChannel(channelId) ?: return false
        return channel.importance != NotificationManager.IMPORTANCE_NONE
    }

    @Suppress("DEPRECATION")
    private fun show(
        arguments: Any?,
        result: MethodChannel.Result,
    ) {
        val request = parseShowRequest(arguments) ?: run {
            invalidArguments(
                result,
                "show requires exactly id, channel, title, body, and route with valid types.",
            )
            return
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            result.error(
                "permission_denied",
                "Notification permission is not granted.",
                null,
            )
            return
        }

        ensureChannels()
        val channelId = when (request.channel) {
            NotificationKind.PRIORITY -> PRIORITY_CHANNEL_ID
            NotificationKind.ACTIVITY -> ACTIVITY_CHANNEL_ID
        }
        val contentIntent = Intent(activity, MainActivity::class.java).apply {
            action = NOTIFICATION_TAP_ACTION
            flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
            putExtra(NOTIFICATION_ROUTE_EXTRA, request.route)
        }
        val pendingIntent = PendingIntent.getActivity(
            activity,
            request.id,
            contentIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(activity, channelId)
        } else {
            Notification.Builder(activity)
        }
        builder
            .setSmallIcon(R.drawable.ic_stat_buzz)
            .setContentTitle(request.title)
            .setContentText(request.body)
            .setStyle(Notification.BigTextStyle().bigText(request.body))
            .setContentIntent(pendingIntent)
            .setAutoCancel(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setCategory(Notification.CATEGORY_MESSAGE)
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            builder.setPriority(
                when (request.channel) {
                    NotificationKind.PRIORITY -> Notification.PRIORITY_DEFAULT
                    NotificationKind.ACTIVITY -> Notification.PRIORITY_LOW
                },
            )
            if (request.channel == NotificationKind.PRIORITY) {
                builder.setDefaults(Notification.DEFAULT_SOUND)
            } else {
                builder.setSound(null)
            }
        }

        notificationManager.notify(request.id, builder.build())
        result.success(null)
    }

    private fun openSettings(result: MethodChannel.Result) {
        val intent = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).apply {
                putExtra(Settings.EXTRA_APP_PACKAGE, activity.packageName)
            }
        } else {
            Intent(
                Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                Uri.fromParts("package", activity.packageName, null),
            )
        }
        try {
            activity.startActivity(intent)
            result.success(null)
        } catch (_: RuntimeException) {
            result.error(
                "settings_unavailable",
                "Notification settings are unavailable.",
                null,
            )
        }
    }

    private fun permissionWasRequested(): Boolean {
        return preferences().getBoolean(PERMISSION_REQUESTED_KEY, false)
    }

    private fun markPermissionRequested() {
        preferences().edit().putBoolean(PERMISSION_REQUESTED_KEY, true).apply()
    }

    private fun preferences() =
        activity.getSharedPreferences(NOTIFICATION_PREFERENCES, Context.MODE_PRIVATE)

    private fun consumeNotificationRoute(intent: Intent): String? {
        val route = intent.getStringExtra(NOTIFICATION_ROUTE_EXTRA)
        intent.removeExtra(NOTIFICATION_ROUTE_EXTRA)
        val component = intent.component
        if (!AndroidNotificationContract.isTrustedTapIntent(
                action = intent.action,
                componentPackage = component?.packageName,
                componentClass = component?.className,
                applicationPackage = activity.packageName,
                mainActivityClass = MainActivity::class.java.name,
            )
        ) {
            return null
        }
        return AndroidNotificationContract.validatedMessageRoute(route)
    }

    private fun invalidArguments(
        result: MethodChannel.Result,
        message: String,
    ) {
        result.error("invalid_arguments", message, null)
    }

    private companion object {
        const val METHOD_CHANNEL = "xyz.block.buzz.mobile/notifications"
        const val PRIORITY_CHANNEL_ID = "buzz_mentions_dms_v1"
        const val ACTIVITY_CHANNEL_ID = "buzz_channel_activity_v1"

        const val GET_STATUS_METHOD = "getStatus"
        const val REQUEST_PERMISSION_METHOD = "requestPermission"
        const val ENSURE_CHANNELS_METHOD = "ensureChannels"
        const val SHOW_METHOD = "show"
        const val OPEN_SETTINGS_METHOD = "openSettings"
        const val GET_INITIAL_ROUTE_METHOD = "getInitialRoute"
        const val NOTIFICATION_TAPPED_METHOD = "notificationTapped"
        const val STATUS_CHANGED_METHOD = "notificationStatusChanged"

        const val NOTIFICATION_PREFERENCES = "buzz_notifications"
        const val PERMISSION_REQUESTED_KEY = "permission_requested"
        const val NOTIFICATION_PERMISSION_REQUEST_CODE = 28031

        val SHOW_ARGUMENT_KEYS = setOf("id", "channel", "title", "body", "route")

        fun parseShowRequest(arguments: Any?): ShowRequest? {
            val payload = arguments as? Map<*, *> ?: return null
            if (payload.keys != SHOW_ARGUMENT_KEYS) return null
            val id = payload["id"] as? Int ?: return null
            val channel = when (payload["channel"] as? String) {
                "priority" -> NotificationKind.PRIORITY
                "activity" -> NotificationKind.ACTIVITY
                else -> return null
            }
            val title = payload["title"] as? String ?: return null
            val body = payload["body"] as? String ?: return null
            val route = payload["route"] as? String ?: return null
            if (AndroidNotificationContract.validatedMessageRoute(route) == null) return null
            return ShowRequest(id, channel, title, body, route)
        }
    }
}

private data class ShowRequest(
    val id: Int,
    val channel: NotificationKind,
    val title: String,
    val body: String,
    val route: String,
)

private enum class NotificationKind {
    PRIORITY,
    ACTIVITY,
}
