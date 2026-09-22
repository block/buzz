package xyz.block.buzz.mobile

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

class BuzzFirebaseMessagingService : FirebaseMessagingService() {
    override fun onRegistered(installationId: String) {
        if (installationId.isBlank()) return
        BuzzAndroidPushStore(applicationContext).saveFcmEndpoint(installationId)
        BuzzAndroidPushPlugin.notifyEndpointChanged(installationId)
    }

    override fun onMessageReceived(message: RemoteMessage) {
        if (message.data[WAKE_KEY] != WAKE_VALUE) return
        showReconnectNotification(applicationContext)
    }

    companion object {
        const val CHANNEL_ID = "buzz_messages"
        const val OPENED_EXTRA = "buzz_push_opened"
        private const val WAKE_KEY = "wake"
        private const val WAKE_VALUE = "1"
        private const val NOTIFICATION_ID = 0x42555a5a

        fun createChannel(context: Context) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_ID,
                    "Messages",
                    NotificationManager.IMPORTANCE_HIGH,
                ).apply {
                    description = "New Buzz messages and mentions"
                    enableVibration(true)
                },
            )
        }

        fun showReconnectNotification(context: Context) {
            createChannel(context)
            val intent = Intent(context, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
                putExtra(OPENED_EXTRA, true)
            }
            val pendingIntent = PendingIntent.getActivity(
                context,
                0,
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val notification = NotificationCompat.Builder(context, CHANNEL_ID)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle("Buzz")
                .setContentText("Reconnect to your relay now")
                .setAutoCancel(true)
                .setPriority(NotificationCompat.PRIORITY_HIGH)
                .setContentIntent(pendingIntent)
                .build()
            try {
                NotificationManagerCompat.from(context).notify(NOTIFICATION_ID, notification)
            } catch (_: SecurityException) {
                // Android 13+ can revoke display permission after token enrollment.
            }
        }
    }
}
