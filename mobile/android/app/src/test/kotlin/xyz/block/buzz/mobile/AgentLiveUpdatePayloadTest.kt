package xyz.block.buzz.mobile

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class AgentLiveUpdatePayloadTest {
    @Test
    fun legacyExpiryTriggerUsesPayloadDeadlineWhenStillFuture() {
        assertEquals(2_000L, legacyExpiryTriggerAt(2_000L, 1_000L))
    }

    @Test
    fun legacyExpiryTriggerSchedulesImmediateCancellationWhenAlreadyExpired() {
        assertEquals(1_001L, legacyExpiryTriggerAt(500L, 1_000L))
    }

    @Test
    fun `parses Flutter method channel values`() {
        val payload = AgentLiveUpdatePayload.from(
            mapOf(
                "title" to "Agent is working",
                "body" to "Working in #agents",
                "activeCount" to 1,
                "startedAtMillis" to 1_722_081_600_000L,
                "expiresAtMillis" to 1_722_110_400_000L,
                "channelId" to "channel-1",
                "messageId" to "message-1",
            ),
        )

        requireNotNull(payload)
        assertEquals("Agent is working", payload.title)
        assertEquals("Working in #agents", payload.body)
        assertEquals(1, payload.activeCount)
        assertEquals(1_722_081_600_000L, payload.startedAtMillis)
        assertEquals(1_722_110_400_000L, payload.expiresAtMillis)
        assertEquals("channel-1", payload.channelId)
        assertEquals("message-1", payload.messageId)
    }

    @Test
    fun `rejects missing or invalid required values`() {
        assertNull(AgentLiveUpdatePayload.from(null))
        assertNull(
            AgentLiveUpdatePayload.from(
                mapOf(
                    "title" to "",
                    "body" to "Working in #agents",
                    "activeCount" to 0,
                    "startedAtMillis" to 1L,
                    "expiresAtMillis" to 30_001,
                    "channelId" to "channel-1",
                ),
            ),
        )
    }
}
