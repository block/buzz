package xyz.block.buzz.mobile

import io.flutter.plugin.common.MethodChannel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse

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

    @Test
    fun `platform failures return a distinct retryable error`() {
        val result = RecordingResult()

        replyWithAgeSignalError(result, IllegalStateException("transient"))

        assertFalse(result.succeeded)
        assertEquals("age_signal_unavailable", result.errorCode)
        assertEquals("The age signal request failed.", result.errorMessage)
        assertEquals("IllegalStateException", result.errorDetails)
    }

    private class RecordingResult : MethodChannel.Result {
        var succeeded = false
        var errorCode: String? = null
        var errorMessage: String? = null
        var errorDetails: Any? = null

        override fun success(result: Any?) {
            succeeded = true
        }

        override fun error(
            errorCode: String,
            errorMessage: String?,
            errorDetails: Any?,
        ) {
            this.errorCode = errorCode
            this.errorMessage = errorMessage
            this.errorDetails = errorDetails
        }

        override fun notImplemented() = Unit
    }
}
