package xyz.block.buzz.mobile

import kotlin.test.Test
import kotlin.test.assertEquals

class AgeSignalPayloadTest {
    @Test
    fun `signal payload contains only status and upper age bound`() {
        assertEquals(
            mapOf(
                "status" to "signal",
                "ageUpper" to 17,
            ),
            ageSignalPayload(17),
        )
        assertEquals(
            mapOf(
                "status" to "signal",
                "ageUpper" to null,
            ),
            ageSignalPayload(null),
        )
    }

    @Test
    fun `no-signal payload contains only status and null upper age bound`() {
        assertEquals(
            mapOf(
                "status" to "noSignal",
                "ageUpper" to null,
            ),
            noAgeSignalPayload(),
        )
    }
}
