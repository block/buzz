package xyz.block.buzz.mobile

import com.google.android.gms.tasks.TaskCompletionSource
import com.google.android.gms.tasks.Tasks
import com.google.android.play.agesignals.AgeSignalsAccessResult
import com.google.android.play.agesignals.AgeSignalsResult
import com.google.android.play.agesignals.model.AgeSignalsStatus
import io.flutter.plugin.common.MethodChannel
import java.util.concurrent.Executor
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class AgeSignalRequestTest {
    private val direct = Executor { it.run() }
    private fun shared() = AgeSignalsAccessResult.builder().setAgeSignalsStatus(AgeSignalsStatus.SHARED).build()
    private fun minor() = AgeSignalsResult.builder().setAgeLower(13).setAgeUpper(17).build()

    @Test fun `shared minor response reaches the Flutter result`() {
        val result = Reply()
        AgeSignalRequest(direct).start(result, { Tasks.forResult(shared()) }, { Tasks.forResult(minor()) })
        assertEquals(listOf<Any?>(mapOf("status" to "signal", "ageUpper" to 17)), result.values)
        assertNull(result.error)
    }

    @Test fun `unknown access does not request or report age`() {
        val result = Reply()
        AgeSignalRequest(direct).start(result, {
            Tasks.forResult(AgeSignalsAccessResult.builder().setAgeSignalsStatus(null).build())
        }, { error("Age lookup must require shared access") })
        assertEquals(listOf<Any?>(mapOf("status" to "noSignal", "ageUpper" to null)), result.values)
    }

    @Test fun `synchronous and asynchronous failures never report a minor`() {
        for (stage in 0..3) {
            val result = Reply()
            val failure = IllegalStateException("injected")
            AgeSignalRequest(direct).start(result, {
                when (stage) {
                    0 -> throw failure
                    1 -> Tasks.forException(failure)
                    else -> Tasks.forResult(shared())
                }
            }, {
                if (stage == 2) throw failure
                Tasks.forException(failure)
            })
            assertEquals(emptyList(), result.values, "stage=$stage")
            assertEquals("age_signal_unavailable", result.error, "stage=$stage")
        }
    }

    @Test fun `retired access callback cannot start age lookup`() {
        val access = TaskCompletionSource<AgeSignalsAccessResult>()
        val request = AgeSignalRequest(direct)
        val result = Reply()
        request.start(result, { access.task }, { error("Retired request must not query age") })
        request.retire()
        access.setResult(shared())
        assertEquals(emptyList(), result.values)
        assertNull(result.error)
    }

    @Test fun `retired minor callback cannot complete a replacement request`() {
        val signal = TaskCompletionSource<AgeSignalsResult>()
        val request = AgeSignalRequest(direct)
        val old = Reply()
        request.start(old, { Tasks.forResult(shared()) }, { signal.task })
        request.retire()
        val replacement = Reply()
        request.start(replacement, { Tasks.forResult(shared()) }, {
            Tasks.forResult(AgeSignalsResult.builder().setAgeLower(18).build())
        })
        signal.setResult(minor())
        assertEquals(emptyList(), old.values)
        assertEquals(listOf<Any?>(mapOf("status" to "signal", "ageUpper" to null)), replacement.values)
    }

    private class Reply : MethodChannel.Result {
        val values = mutableListOf<Any?>()
        var error: String? = null
        override fun success(result: Any?) { values.add(result) }
        override fun error(errorCode: String, errorMessage: String?, errorDetails: Any?) { error = errorCode }
        override fun notImplemented() = Unit
    }
}
