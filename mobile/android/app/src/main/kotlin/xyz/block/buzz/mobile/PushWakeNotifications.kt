package xyz.block.buzz.mobile

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.core.app.NotificationChannelCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person

/** Where a notification's tap leads: the arguments of the app's message deep link. */
internal object NotificationOpen {
    const val ACTION = "xyz.block.buzz.mobile.OPEN_NOTIFICATION"
    private const val COMMUNITY_ID = "communityId"
    private const val CHANNEL_ID = "channelId"
    private const val EVENT_ID = "eventId"
    private const val THREAD_ROOT_ID = "threadRootId"

    /**
     * [identity] tells one notification's tap apart from another's: a
     * PendingIntent matches by action, data and component, not by extras, so
     * without it two conversations would share one and open the same target.
     */
    fun intent(context: Context, identity: String, target: Map<String, String>): Intent =
        Intent(context, MainActivity::class.java)
            .setAction(ACTION)
            .setData(Uri.fromParts(IDENTITY_SCHEME, identity, null))
            // A picker or Custom Tab in Buzz's task can sit above MainActivity.
            // These deliver the tap to the running MainActivity's onNewIntent
            // instead of starting a second one, or only raising the task.
            .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .apply { target.forEach { (key, value) -> putExtra(key, value) } }

    private const val IDENTITY_SCHEME = "buzz-notification"

    /** The arguments `buzz/push` hands Dart, or null when [intent] is not a notification tap. */
    fun arguments(intent: Intent?): Map<String, String>? {
        if (intent?.action != ACTION) return null
        return listOf(COMMUNITY_ID, CHANNEL_ID, EVENT_ID, THREAD_ROOT_ID)
            .mapNotNull { key -> intent.getStringExtra(key)?.let { key to it } }
            .toMap()
    }
}

internal enum class WakeCategory(val dartName: String, val channelId: String) {
    DIRECT_MESSAGE("directMessage", "direct_messages"),
    MENTION("mention", "mentions"),
    ;

    companion object {
        fun fromDart(name: Any?): WakeCategory =
            entries.firstOrNull { it.dartName == name }
                ?: throw IllegalArgumentException("Unknown notification category: $name")
    }
}

/** One message a wake found, as Dart's `pushWakeChannelPayload` sends it. */
internal data class WakeNotification(
    val eventId: String,
    val createdAtMillis: Long,
    val title: String,
    val body: String,
    val conversationId: String,
    val conversationTitle: String?,
    val category: WakeCategory,
    val target: Map<String, String>,
) {
    companion object {
        fun fromDart(value: Any?): WakeNotification {
            val map = value as? Map<*, *> ?: throw IllegalArgumentException("Not a notification")
            fun text(key: String) =
                map[key] as? String ?: throw IllegalArgumentException("Missing $key")
            val target = map["target"] as? Map<*, *> ?: throw IllegalArgumentException("Missing target")
            return WakeNotification(
                eventId = text("eventId"),
                createdAtMillis = (map["createdAtMillis"] as? Number)?.toLong()
                    ?: throw IllegalArgumentException("Missing createdAtMillis"),
                title = text("title"),
                body = text("body"),
                conversationId = text("conversationId"),
                conversationTitle = map["conversationTitle"] as? String,
                category = WakeCategory.fromDart(map["category"]),
                target = target.entries.associate { (key, value) ->
                    key as String to value as String
                },
            )
        }
    }
}

/**
 * Posts what a push wake found: one `MessagingStyle` notification per
 * conversation, and the generic one for a wake that found nothing in time.
 */
internal class WakeNotifier(private val context: Context) {
    private val manager = NotificationManagerCompat.from(context)

    companion object {
        const val OTHER_CHANNEL = "other"
        private const val GROUP_KEY = "xyz.block.buzz.mobile.conversations"
        private const val CONVERSATION_ID = 1
        private const val GENERIC_ID = 2
        private const val GENERIC_TAG = "generic"
        private const val EVENT_ID_EXTRA = "eventId"

        private fun conversationTag(conversationId: String) = "conversation:$conversationId"
    }

    fun createChannels() {
        fun channel(id: String, name: Int, importance: Int) =
            NotificationChannelCompat.Builder(id, importance)
                .setName(context.getString(name))
                .build()
        manager.createNotificationChannelsCompat(
            listOf(
                channel(
                    WakeCategory.DIRECT_MESSAGE.channelId,
                    R.string.notification_channel_direct_messages,
                    NotificationManagerCompat.IMPORTANCE_HIGH,
                ),
                channel(
                    WakeCategory.MENTION.channelId,
                    R.string.notification_channel_mentions,
                    NotificationManagerCompat.IMPORTANCE_HIGH,
                ),
                channel(
                    OTHER_CHANNEL,
                    R.string.notification_channel_other,
                    NotificationManagerCompat.IMPORTANCE_DEFAULT,
                ),
            ),
        )
    }

    /**
     * Posts each conversation's messages, added to what its notification
     * already shows. [silent] keeps them from alerting, for a result that
     * replaces the generic notification, which already did.
     */
    fun postConversations(notifications: List<WakeNotification>, silent: Boolean) {
        for ((conversationId, messages) in notifications.groupBy { it.conversationId }) {
            val oldestFirst = messages.sortedWith(compareBy({ it.createdAtMillis }, { it.eventId }))
            val newest = oldestFirst.last()
            val tag = conversationTag(conversationId)
            val style = existingStyle(tag)
                ?: NotificationCompat.MessagingStyle(Person.Builder().setName(context.getString(R.string.notification_you)).build())
            // A wake that could not read the channel's name sends none, so
            // keep the one an earlier wake showed. Mentions are in channels and
            // DMs are one to one, whether or not a name arrived.
            style.conversationTitle = newest.conversationTitle ?: style.conversationTitle
            style.isGroupConversation = newest.category == WakeCategory.MENTION
            val shown = style.messages.mapNotNull { it.extras.getString(EVENT_ID_EXTRA) }.toSet()
            for (message in oldestFirst) {
                if (message.eventId in shown) continue
                style.addMessage(
                    NotificationCompat.MessagingStyle.Message(
                        message.body,
                        message.createdAtMillis,
                        Person.Builder().setName(message.title).setKey(message.title).build(),
                    ).also { it.extras.putString(EVENT_ID_EXTRA, message.eventId) },
                )
            }
            val tap = PendingIntent.getActivity(
                context,
                0,
                NotificationOpen.intent(context, tag, newest.target),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val notification = NotificationCompat.Builder(context, newest.category.channelId)
                .setSmallIcon(R.drawable.ic_stat_buzz)
                .setStyle(style)
                .setCategory(NotificationCompat.CATEGORY_MESSAGE)
                .setGroup(GROUP_KEY)
                .setWhen(newest.createdAtMillis)
                .setAutoCancel(true)
                .setContentIntent(tap)
                // Only a group's summary alerts, and conversations here have
                // none, so this keeps the notification from alerting.
                .setGroupAlertBehavior(
                    if (silent) NotificationCompat.GROUP_ALERT_SUMMARY else NotificationCompat.GROUP_ALERT_ALL,
                )
                .build()
            manager.notify(tag, CONVERSATION_ID, notification)
        }
    }

    fun postGeneric() {
        val tap = PendingIntent.getActivity(
            context,
            GENERIC_ID,
            context.packageManager.getLaunchIntentForPackage(context.packageName)
                ?: throw IllegalStateException("No launch intent"),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        manager.notify(
            GENERIC_TAG,
            GENERIC_ID,
            NotificationCompat.Builder(context, OTHER_CHANNEL)
                .setSmallIcon(R.drawable.ic_stat_buzz)
                .setContentTitle(context.getString(R.string.app_name))
                .setContentText(context.getString(R.string.notification_new_activity))
                .setCategory(NotificationCompat.CATEGORY_MESSAGE)
                .setAutoCancel(true)
                .setContentIntent(tap)
                .build(),
        )
    }

    fun cancelGeneric() = manager.cancel(GENERIC_TAG, GENERIC_ID)

    private fun existingStyle(tag: String): NotificationCompat.MessagingStyle? =
        manager.activeNotifications
            .firstOrNull { it.tag == tag && it.id == CONVERSATION_ID }
            ?.notification
            ?.let { NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(it) }
}
