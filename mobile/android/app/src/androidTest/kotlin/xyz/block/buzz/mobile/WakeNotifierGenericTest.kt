package xyz.block.buzz.mobile

import android.Manifest
import android.os.Build
import android.os.SystemClock
import android.service.notification.StatusBarNotification
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import kotlin.test.fail

/**
 * The generic notification against the real notification manager: a card
 * kept for a lost message survives later wakes, and one the user cleared
 * stays cleared.
 */
@RunWith(AndroidJUnit4::class)
class WakeNotifierGenericTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val manager = NotificationManagerCompat.from(context)
    private val notifier = WakeNotifier(context)

    @Before fun setUp() {
        if (Build.VERSION.SDK_INT >= 33) {
            InstrumentationRegistry.getInstrumentation().uiAutomation
                .grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
        }
        notifier.createChannels()
        manager.cancelAll()
    }

    @After fun tearDown() = manager.cancelAll()

    @Test fun lostGenericJourney() {
        // A clean wake clears an unmarked generic, as before.
        notifier.postGeneric()
        awaitGeneric(Generic.SHOWING)
        notifier.cancelGeneric()
        awaitGeneric(Generic.NONE)

        // The 2-second card is still up when the result reports a lost
        // message: it is kept, and later clean wakes leave it.
        notifier.postGeneric()
        val timerCard = awaitGeneric(Generic.SHOWING)
        paceUpdate()
        assertTrue(notifier.keepGeneric(shownThisWake = true, silent = true))
        awaitRepost(timerCard, Generic.LOST)
        notifier.cancelGeneric()
        drain()
        assertEquals(Generic.LOST, generic())

        // A later wake's 2-second card re-posts it and keeps the mark.
        val kept = activeGeneric()!!
        paceUpdate()
        notifier.postGeneric()
        awaitRepost(kept, Generic.LOST)

        // The user clears it.
        clearGeneric()
        awaitGeneric(Generic.NONE)

        // The user clears the 2-second card before the result reports a lost
        // message: it is not posted again.
        notifier.postGeneric()
        awaitGeneric(Generic.SHOWING)
        clearGeneric()
        awaitGeneric(Generic.NONE)
        assertFalse(notifier.keepGeneric(shownThisWake = true, silent = true))
        drain()
        assertEquals(Generic.NONE, generic())

        // Nothing posted this wake yet: the kept card is posted.
        assertTrue(notifier.keepGeneric(shownThisWake = false, silent = false))
        awaitGeneric(Generic.LOST)
    }

    private enum class Generic { NONE, SHOWING, LOST }

    // As a swipe or tap does: the platform cancels the card, not the handler.
    private fun clearGeneric() = manager.cancel("generic", 2)

    // The notification service applies posts and cancels asynchronously, so
    // each step waits, within a bound, for the state it expects.
    private fun awaitGeneric(expected: Generic): StatusBarNotification? {
        await("generic $expected, was ${generic()}") { generic() == expected }
        return activeGeneric()
    }

    // Waits for a new post of the card, so the state read is the re-post's
    // and not the record it replaced.
    private fun awaitRepost(previous: StatusBarNotification?, expected: Generic) {
        await("a re-post of the generic") {
            activeGeneric()?.let { it.postTime != previous?.postTime } == true
        }
        assertEquals(expected, generic())
    }

    // Android silently drops an update to a showing card while the app
    // posts faster than max_notification_enqueue_rate (5 a second), and the
    // estimate carries over between runs. 700 ms with no post keeps the
    // rate under that, so quick reruns don't shed the test's updates.
    private fun paceUpdate() = SystemClock.sleep(700)

    // Before asserting that nothing changed: a barrier card posted and
    // cancelled after the step is applied after it too.
    private fun drain() {
        manager.notify(
            "barrier",
            3,
            NotificationCompat.Builder(context, WakeNotifier.OTHER_CHANNEL)
                .setSmallIcon(android.R.drawable.stat_notify_chat)
                .build(),
        )
        await("the barrier card") { manager.activeNotifications.any { it.tag == "barrier" } }
        manager.cancel("barrier", 3)
        await("the barrier card to clear") { manager.activeNotifications.none { it.tag == "barrier" } }
    }

    private fun await(what: String, condition: () -> Boolean) {
        val deadline = SystemClock.uptimeMillis() + 5_000
        while (!condition()) {
            if (SystemClock.uptimeMillis() > deadline) fail("timed out waiting for $what")
            Thread.sleep(20)
        }
    }

    private fun activeGeneric(): StatusBarNotification? {
        val active = manager.activeNotifications.filter { it.tag == "generic" && it.id == 2 }
        assertTrue(active.size <= 1)
        return active.firstOrNull()
    }

    private fun generic(): Generic {
        val card = activeGeneric() ?: return Generic.NONE
        return if (card.notification.extras.getBoolean("xyz.block.buzz.mobile.lost")) Generic.LOST else Generic.SHOWING
    }
}
