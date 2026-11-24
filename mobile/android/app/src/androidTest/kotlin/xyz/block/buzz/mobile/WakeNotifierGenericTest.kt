package xyz.block.buzz.mobile

import android.Manifest
import android.os.Build
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
        settle { notifier.postGeneric() }
        assertEquals(Generic.SHOWING, generic())
        settle { notifier.cancelGeneric() }
        assertEquals(Generic.NONE, generic())

        // The 2-second card is still up when the result reports a lost
        // message: it is kept, and later clean wakes leave it.
        settle { notifier.postGeneric() }
        assertTrue(settle { notifier.keepGeneric(shownThisWake = true, silent = true) })
        assertEquals(Generic.LOST, generic())
        settle { notifier.cancelGeneric() }
        assertEquals(Generic.LOST, generic())

        // A later wake's 2-second card re-posts it and keeps the mark.
        settle { notifier.postGeneric() }
        assertEquals(Generic.LOST, generic())

        // The user clears it.
        settle { clearGeneric() }
        assertEquals(Generic.NONE, generic())

        // The user clears the 2-second card before the result reports a lost
        // message: it is not posted again.
        settle { notifier.postGeneric() }
        settle { clearGeneric() }
        assertFalse(settle { notifier.keepGeneric(shownThisWake = true, silent = true) })
        assertEquals(Generic.NONE, generic())

        // Nothing posted this wake yet: the kept card is posted.
        assertTrue(settle { notifier.keepGeneric(shownThisWake = false, silent = false) })
        assertEquals(Generic.LOST, generic())
    }

    private enum class Generic { NONE, SHOWING, LOST }

    // As a swipe or tap does: the platform cancels the card, not the handler.
    private fun clearGeneric() = manager.cancel("generic", 2)

    // The notification service applies posts and cancels asynchronously.
    private fun <T> settle(action: () -> T): T = action().also { Thread.sleep(500) }

    private fun generic(): Generic {
        val active = manager.activeNotifications.filter { it.tag == "generic" && it.id == 2 }
        assertTrue(active.size <= 1)
        val card = active.firstOrNull() ?: return Generic.NONE
        return if (card.notification.extras.getBoolean("xyz.block.buzz.mobile.lost")) Generic.LOST else Generic.SHOWING
    }
}
