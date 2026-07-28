package xyz.block.buzz.mobile

import android.Manifest
import android.app.Activity
import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle

private const val CHANNEL_ID = "buzz_agent_live_updates"
private const val NOTIFICATION_TAG = "buzz_agent_live_update"
private const val NOTIFICATION_ID = 24_200
private const val NOTIFICATION_PERMISSION_REQUEST_CODE = 24_201
private const val CONTENT_INTENT_REQUEST_CODE = 24_202
private const val EXPIRY_INTENT_REQUEST_CODE = 24_203
private const val EXPIRY_INTENT_ACTION = "xyz.block.buzz.mobile.AGENT_LIVE_UPDATE_EXPIRED"
private const val PREFERENCES_NAME = "buzz_agent_live_updates"
private const val PERMISSION_REQUESTED_KEY = "notification_permission_requested"
private const val REQUEST_PROMOTED_ONGOING_EXTRA = "android.requestPromotedOngoing"

internal class AgentLiveUpdateManager(
    private val activity: Activity,
) {
    private val notificationManager =
        activity.getSystemService(NotificationManager::class.java)
    private val alarmManager =
        activity.getSystemService(AlarmManager::class.java)
    private val preferences =
        activity.getSharedPreferences(PREFERENCES_NAME, Activity.MODE_PRIVATE)

    private var pendingPayload: AgentLiveUpdatePayload? = null
    private var permissionRequestInFlight = false

    fun show(payload: AgentLiveUpdatePayload): Boolean {
        pendingPayload = payload
        if (!canPostNotifications()) {
            requestNotificationPermissionOnce()
            return false
        }

        post(payload)
        pendingPayload = null
        return true
    }

    fun dismiss() {
        pendingPayload = null
        cancelLegacyExpiry()
        notificationManager.cancel(NOTIFICATION_TAG, NOTIFICATION_ID)
    }

    fun onRequestPermissionsResult(
        requestCode: Int,
        grantResults: IntArray,
    ) {
        if (requestCode != NOTIFICATION_PERMISSION_REQUEST_CODE) return
        permissionRequestInFlight = false
        if (grantResults.firstOrNull() != PackageManager.PERMISSION_GRANTED) {
            pendingPayload = null
            return
        }

        pendingPayload?.let(::post)
        pendingPayload = null
    }

    private fun canPostNotifications(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED

    private fun requestNotificationPermissionOnce() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        if (permissionRequestInFlight) return
        if (preferences.getBoolean(PERMISSION_REQUESTED_KEY, false)) return

        permissionRequestInFlight = true
        preferences.edit().putBoolean(PERMISSION_REQUESTED_KEY, true).apply()
        activity.requestPermissions(
            arrayOf(Manifest.permission.POST_NOTIFICATIONS),
            NOTIFICATION_PERMISSION_REQUEST_CODE,
        )
    }

    private fun post(payload: AgentLiveUpdatePayload) {
        createChannel()

        val publicNotification = newNotificationBuilder()
            .setSmallIcon(R.drawable.ic_agent_live_update)
            .setContentTitle(
                activity.resources.getQuantityString(
                    R.plurals.agent_activity_public_title,
                    payload.activeCount,
                    payload.activeCount,
                ),
            )
            .setContentText(activity.getString(R.string.agent_activity_public_body))
            .setContentIntent(contentIntent(payload))
            .setCategory(Notification.CATEGORY_PROGRESS)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(true)
            .setWhen(payload.startedAtMillis)
            .setUsesChronometer(true)
            .build()

        val notification = newNotificationBuilder()
            .setSmallIcon(R.drawable.ic_agent_live_update)
            .setContentTitle(payload.title)
            .setContentText(payload.body)
            .setStyle(Notification.BigTextStyle().bigText(payload.body))
            .setContentIntent(contentIntent(payload))
            .setPublicVersion(publicNotification)
            .setCategory(Notification.CATEGORY_PROGRESS)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(true)
            .setWhen(payload.startedAtMillis)
            .setUsesChronometer(true)
            .addExtras(
                Bundle().apply {
                    putBoolean(REQUEST_PROMOTED_ONGOING_EXTRA, true)
                },
            )
            .apply {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    val remainingLifetime =
                        (payload.expiresAtMillis - System.currentTimeMillis()).coerceAtLeast(1L)
                    setTimeoutAfter(remainingLifetime)
                }
            }
            .build()

        notificationManager.notify(NOTIFICATION_TAG, NOTIFICATION_ID, notification)
        scheduleLegacyExpiry(payload.expiresAtMillis)
    }

    private fun newNotificationBuilder(): Notification.Builder =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(activity, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(activity)
        }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val channel = NotificationChannel(
            CHANNEL_ID,
            activity.getString(R.string.agent_activity_channel_name),
            NotificationManager.IMPORTANCE_DEFAULT,
        ).apply {
            description = activity.getString(R.string.agent_activity_channel_description)
            lockscreenVisibility = Notification.VISIBILITY_PUBLIC
            enableVibration(false)
            setSound(null, null)
            setShowBadge(false)
        }
        notificationManager.createNotificationChannel(channel)
    }

    private fun scheduleLegacyExpiry(expiresAtMillis: Long) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            cancelLegacyExpiry()
            return
        }
        alarmManager.setExactAndAllowWhileIdle(
            AlarmManager.RTC_WAKEUP,
            legacyExpiryTriggerAt(expiresAtMillis, System.currentTimeMillis()),
            expiryIntent(),
        )
    }

    private fun cancelLegacyExpiry() {
        val intent = expiryIntent()
        alarmManager.cancel(intent)
        intent.cancel()
    }

    private fun expiryIntent(): PendingIntent =
        PendingIntent.getBroadcast(
            activity,
            EXPIRY_INTENT_REQUEST_CODE,
            Intent(activity, AgentLiveUpdateExpiryReceiver::class.java).apply {
                action = EXPIRY_INTENT_ACTION
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

    private fun contentIntent(payload: AgentLiveUpdatePayload): PendingIntent {
        val messageId = payload.messageId
        val intent = if (messageId != null) {
            Intent(Intent.ACTION_VIEW).apply {
                setClass(activity, MainActivity::class.java)
                data = Uri.Builder()
                    .scheme("buzz")
                    .authority("message")
                    .appendQueryParameter("channel", payload.channelId)
                    .appendQueryParameter("id", messageId)
                    .build()
            }
        } else {
            activity.packageManager.getLaunchIntentForPackage(activity.packageName)
                ?: Intent(activity, MainActivity::class.java)
        }.apply {
            flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
        }
        return PendingIntent.getActivity(
            activity,
            CONTENT_INTENT_REQUEST_CODE,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }
}

internal fun legacyExpiryTriggerAt(
    expiresAtMillis: Long,
    nowMillis: Long,
): Long = expiresAtMillis.coerceAtLeast(nowMillis + 1L)

internal class AgentLiveUpdateExpiryReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        if (intent.action != EXPIRY_INTENT_ACTION) return
        context
            .getSystemService(NotificationManager::class.java)
            .cancel(NOTIFICATION_TAG, NOTIFICATION_ID)
    }
}
