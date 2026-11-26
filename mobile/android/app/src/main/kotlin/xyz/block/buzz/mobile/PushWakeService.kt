package xyz.block.buzz.mobile

import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

/**
 * Receives the push gateway's wake. The wake carries nothing the app reads: the
 * message is fetched from the relay. FCM calls this off the main thread and
 * keeps the process alive until it returns.
 */
class PushWakeService : FirebaseMessagingService() {
    override fun onMessageReceived(message: RemoteMessage) = PushWake.handle(applicationContext)

    /**
     * FCM dropped wakes it held for this phone, so any number of messages may
     * be waiting. A wake means "sync now", so this is one.
     */
    override fun onDeletedMessages() = PushWake.handle(applicationContext)
}
