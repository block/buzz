package xyz.block.buzz.mobile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class HuddleActiveTalkerSelectorTest {
    @Test
    fun `sixteen senders retain exactly the active talker capacity`() {
        val selector = HuddleActiveTalkerSelector(capacity = 15)

        repeat(15) { peer -> assertNull(selector.activate(peer, -20)) }
        assertEquals(0, selector.activate(15, -20))
        assertEquals((1..15).toSet(), selector.indices())
    }

    @Test
    fun `recent activity wins and an evicted peer can reactivate`() {
        val selector = HuddleActiveTalkerSelector(capacity = 2)

        selector.activate(4, -40)
        selector.activate(2, -20)
        selector.activate(4, -10)
        assertEquals(2, selector.activate(9, -30))
        assertEquals(setOf(4, 9), selector.indices())

        assertEquals(4, selector.activate(2, -15))
        assertEquals(setOf(2, 9), selector.indices())
    }

    @Test
    fun `stable peer index breaks equal activity ties`() {
        val selector = HuddleActiveTalkerSelector(capacity = 1)

        selector.activate(7, -30)
        assertEquals(7, selector.activate(3, -30))
        assertEquals(setOf(3), selector.indices())
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
        assertNull(selector.activate(7, -20))
    }
}
