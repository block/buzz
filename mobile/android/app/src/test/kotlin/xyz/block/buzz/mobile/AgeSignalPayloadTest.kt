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
    fun `platform failures return a distinct error for the fail-open caller`() {
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
    fun `transient integration and unknown Play errors remain distinguishable`() {
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

    @Test
    fun `invalid and contradictory ranges do not report an underage upper bound`() {
        for ((lower, upper) in listOf(-1 to 17, 18 to 17, 0 to -1, Int.MAX_VALUE to 17)) {
            assertNull(ageSignalPayload(upper, lower)["ageUpper"])
        }
        assertEquals(17, ageSignalPayload(17, 13)["ageUpper"])
        assertEquals(18, ageSignalPayload(18, 18)["ageUpper"])
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
