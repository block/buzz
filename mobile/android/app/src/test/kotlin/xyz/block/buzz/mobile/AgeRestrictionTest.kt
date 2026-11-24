package xyz.block.buzz.mobile

import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class AgeRestrictionTest {
    @AfterTest
    fun release() = AgeRestriction.release()

    @Test
    fun restrictionCancelsPostedNotificationsAndBlocksPostingUntilReleased() {
        val posted = mutableListOf<String>()
        AgeRestriction.unlessRestricted { posted += "before" }

        AgeRestriction.restrict { posted.clear() }
        AgeRestriction.unlessRestricted { posted += "restricted" }

        assertTrue(AgeRestriction.isRestricted)
        assertEquals(emptyList(), posted)

        AgeRestriction.release()
        AgeRestriction.unlessRestricted { posted += "released" }

        assertFalse(AgeRestriction.isRestricted)
        assertEquals(listOf("released"), posted)
    }

    @Test
    fun postingInProgressFinishesBeforeRestrictionCancels() {
        val events = mutableListOf<String>()
        val posting = java.util.concurrent.CountDownLatch(1)
        val restricting = Thread {
            posting.await()
            AgeRestriction.restrict { events += "cancelAll" }
        }
        restricting.start()

        AgeRestriction.unlessRestricted {
            posting.countDown()
            // The restricting thread is now waiting for this post to finish.
            Thread.sleep(50)
            events += "post"
        }
        restricting.join()

        assertEquals(listOf("post", "cancelAll"), events)
    }
}
