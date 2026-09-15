package xyz.block.buzz.mobile

import com.google.android.play.agesignals.AgeSignalsException
import com.google.android.play.agesignals.model.AgeSignalsErrorCode
import io.flutter.plugin.common.MethodChannel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import kotlin.test.assertNull

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

    @Test
    fun `unavailable Play environments return no signal`() {
        for (code in listOf(
            AgeSignalsErrorCode.API_NOT_AVAILABLE,
            AgeSignalsErrorCode.PLAY_STORE_NOT_FOUND,
            AgeSignalsErrorCode.PLAY_SERVICES_NOT_FOUND,
            AgeSignalsErrorCode.PLAY_STORE_VERSION_OUTDATED,
            AgeSignalsErrorCode.PLAY_SERVICES_VERSION_OUTDATED,
            AgeSignalsErrorCode.APP_NOT_OWNED,
        )) {
            val result = RecordingResult()
            replyWithAgeSignalError(result, AgeSignalsException(code))
            assertTrue(result.succeeded, "code=$code")
            assertEquals(mapOf("status" to "noSignal", "ageUpper" to null), result.payload)
            assertNull(result.errorCode)
        }
    }

    @Test
    fun `transient integration and unknown Play errors stay gated`() {
        for (code in listOf(
            AgeSignalsErrorCode.NETWORK_ERROR,
            AgeSignalsErrorCode.CANNOT_BIND_TO_SERVICE,
            AgeSignalsErrorCode.CLIENT_TRANSIENT_ERROR,
            AgeSignalsErrorCode.SDK_VERSION_OUTDATED,
            AgeSignalsErrorCode.INTERNAL_ERROR,
            -999,
        )) {
            val result = RecordingResult()
            replyWithAgeSignalError(result, AgeSignalsException(code))
            assertFalse(result.succeeded, "code=$code")
            assertEquals("age_signal_unavailable", result.errorCode)
        }
    }

    private class RecordingResult : MethodChannel.Result {
        var succeeded = false
        var payload: Any? = null
        var errorCode: String? = null
        var errorMessage: String? = null
        var errorDetails: Any? = null

        override fun success(result: Any?) {
            succeeded = true
            payload = result
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
