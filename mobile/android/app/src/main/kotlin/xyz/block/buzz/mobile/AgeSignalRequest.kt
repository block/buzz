package xyz.block.buzz.mobile

import com.google.android.gms.tasks.Task
import com.google.android.gms.tasks.TaskExecutors
import com.google.android.play.agesignals.AgeSignalsAccessResult
import com.google.android.play.agesignals.AgeSignalsResult
import com.google.android.play.agesignals.model.AgeSignalsStatus
import io.flutter.plugin.common.MethodChannel
import java.util.concurrent.Executor

/** Owns one native request. Calls and callbacks run on the main thread. */
internal class AgeSignalRequest(private val executor: Executor = TaskExecutors.MAIN_THREAD) {
    private var generation = 0
    private var pending: MethodChannel.Result? = null

    fun retire() {
        generation += 1
        pending = null
    }

    fun start(
        result: MethodChannel.Result,
        requestAccess: () -> Task<AgeSignalsAccessResult>,
        checkAge: () -> Task<AgeSignalsResult>,
    ) {
        if (pending != null) {
            result.error("age_signal_in_flight", "An age signal request is already active.", null)
            return
        }
        val request = ++generation
        pending = result
        fun current() = generation == request && pending === result
        fun complete(reply: () -> Unit) {
            if (!current()) return
            pending = null
            reply()
        }
        fun fail(error: Exception) = complete { replyWithAgeSignalError(result, error) }
        try {
            requestAccess()
                .addOnSuccessListener(executor) { access ->
                    if (!current()) return@addOnSuccessListener
                    if (access.ageSignalsStatus() != AgeSignalsStatus.SHARED) {
                        complete { result.success(noAgeSignalPayload()) }
                        return@addOnSuccessListener
                    }
                    try {
                        checkAge()
                            .addOnSuccessListener(executor) { signal ->
                                complete { result.success(ageSignalPayload(signal.ageUpper(), signal.ageLower())) }
                            }
                            .addOnFailureListener(executor, ::fail)
                    } catch (error: Exception) {
                        fail(error)
                    }
                }
                .addOnFailureListener(executor, ::fail)
        } catch (error: Exception) {
            fail(error)
        }
    }
}
