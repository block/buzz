package xyz.block.buzz.mobile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

class HuddleActiveTalkerSelectorTest {
    @Test
    fun `sixteen senders retain exactly the active talker capacity`() {
        val selector = HuddleActiveTalkerSelector(capacity = 15)

        repeat(15) { peer ->
            assertEquals(
                HuddleTalkerSelection(accepted = true, evictedPeerIndex = null),
                selector.activate(peer, -20),
            )
        }
        val rejected = selector.activate(15, -20)
        assertEquals(
            HuddleTalkerSelection(accepted = false, evictedPeerIndex = null),
            rejected,
        )
        assertNull(rejected.allocateIfAccepted { Any() })
        assertEquals((0..14).toSet(), selector.indices())
    }

    @Test
    fun `recent activity wins and an evicted peer can reactivate`() {
        val selector = HuddleActiveTalkerSelector(capacity = 2)

        selector.activate(4, -40)
        selector.activate(2, -20)
        selector.activate(4, -10)
        assertEquals(
            HuddleTalkerSelection(accepted = true, evictedPeerIndex = 2),
            selector.activate(9, -5),
        )
        assertEquals(setOf(4, 9), selector.indices())

        assertEquals(
            HuddleTalkerSelection(accepted = true, evictedPeerIndex = 4),
            selector.activate(2, -1),
        )
        assertEquals(setOf(2, 9), selector.indices())
    }

    @Test
    fun `equal level does not replace selected peer`() {
        val selector = HuddleActiveTalkerSelector(capacity = 1)

        selector.activate(7, -30)
        assertEquals(
            HuddleTalkerSelection(accepted = false, evictedPeerIndex = null),
            selector.activate(3, -30),
        )
        assertEquals(setOf(7), selector.indices())
    }

    @Test
    fun `equal level packets do not churn selected peers`() {
        val selector = HuddleActiveTalkerSelector(capacity = 2)

        selector.activate(4, -127)
        selector.activate(2, -127)
        assertEquals(
            HuddleTalkerSelection(accepted = false, evictedPeerIndex = null),
            selector.activate(9, -127),
        )
        assertFalse(9 in selector.indices())
        assertEquals(setOf(2, 4), selector.indices())
    }

    @Test
    fun `louder peer replaces the quietest selected peer`() {
        val selector = HuddleActiveTalkerSelector(capacity = 2)

        selector.activate(4, -40)
        selector.activate(2, -20)
        assertEquals(
            HuddleTalkerSelection(accepted = true, evictedPeerIndex = 4),
            selector.activate(9, -30),
        )
        assertEquals(setOf(2, 9), selector.indices())
    }

    @Test
    fun `inactive selected peer yields its slot to a quieter speaker`() {
        var nowMs = 0L
        val selector = HuddleActiveTalkerSelector(
            capacity = 2,
            nowMs = { nowMs },
            inactivityTimeoutMs = 1_000,
        )

        selector.activate(4, -10)
        nowMs = 999
        selector.activate(2, -5)
        nowMs = 1_000

        assertEquals(
            HuddleTalkerSelection(accepted = true, evictedPeerIndex = 4),
            selector.activate(9, -30),
        )
        assertEquals(setOf(2, 9), selector.indices())
    }

    @Test
    fun `jitter queue reorders packets and rejects stale duplicates`() {
        val queue = HuddlePacketJitterQueue(capacity = 3, startPackets = 2)
        queue.enqueue(packet(11))
        queue.enqueue(packet(10))
        assertEquals(listOf(10, 11), queue.sequences())
        assertEquals(10, queue.drainOne()?.sequence)
        queue.enqueue(packet(10))
        queue.enqueue(packet(12))
        assertEquals(listOf(11, 12), queue.sequences())
    }

    private fun packet(sequence: Int) = HuddleRemoteOpusPacket(
        peerIndex = 1,
        sequence = sequence,
        timestamp48k = sequence.toLong() * 960,
        levelDbov = -20,
        opus = byteArrayOf(1),
    )

    @Test
    fun `remove clears the slot without allocating roster-only peers`() {
        val selector = HuddleActiveTalkerSelector(capacity = 2)

        assertEquals(emptySet<Int>(), selector.indices())
        selector.activate(7, -20)
        selector.remove(7)
        assertEquals(emptySet<Int>(), selector.indices())
        assertEquals(
            HuddleTalkerSelection(accepted = true, evictedPeerIndex = null),
            selector.activate(7, -20),
        )
    }
}
