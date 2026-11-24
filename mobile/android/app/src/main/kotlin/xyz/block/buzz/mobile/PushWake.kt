package xyz.block.buzz.mobile

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import androidx.core.app.NotificationManagerCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import io.flutter.FlutterInjector
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.embedding.engine.dart.DartExecutor
import io.flutter.plugin.common.MethodChannel
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Turns a push wake into notifications.
 *
 * A wake carries nothing about any message. When the app is in the foreground
 * the handler does nothing, since the running app already shows new messages.
 * Otherwise it starts the app's Dart code in a background engine, which reads
 * the relays and answers with every message worth a notification, and posts
 * them directly: the category, and so the channel the user may have silenced,
 * is known only after the fetch.
 *
 * If nothing has been posted after [GENERIC_AFTER_MS] the handler posts a
 * generic notification in the "Other" channel, which the result then replaces
 * silently, or removes when it finds nothing. A watchdog ends the work at
 * [WATCHDOG_MS] whatever Dart does, inside the time FCM gives a handler, and
 * leaves the generic notification when no result came.
 *
 * One run exists per process. A wake that arrives while it runs is passed to
 * the Dart code, which folds it into the run in progress.
 */
internal object PushWake {
    private const val TAG = "BuzzPushWake"
    const val GENERIC_AFTER_MS = 2_000L
    const val WATCHDOG_MS = 8_000L

    /**
     * Watchdog time held back from Dart's lookup: its claim grace, the answer's
     * trip back and posting. A slow engine start then shortens the lookup
     * instead of letting the watchdog end it after messages were recorded as
     * shown but before they were posted.
     */
    const val RESULT_RESERVE_MS = 1_000L
    private const val ENTRYPOINT = "androidPushWakeMain"
    private const val CHANNEL = "buzz/push_wake"

    private val main = Handler(Looper.getMainLooper())

    // Main thread only.
    private var run: Run? = null

    /**
     * Handles a wake and returns when it is handled, so the process stays
     * alive for it. Call it off the main thread, as FCM does.
     */
    fun handle(context: Context) {
        check(Looper.myLooper() != Looper.getMainLooper()) { "Handle a wake off the main thread" }
        val app = context.applicationContext
        val handled = CountDownLatch(1)
        main.post { begin(app, handled) }
        // The watchdog ends the run; the extra time covers only its own posting.
        handled.await(WATCHDOG_MS + 2_000, TimeUnit.MILLISECONDS)
    }

    private fun begin(context: Context, handled: CountDownLatch) {
        // Declined permission costs nothing else: nothing is fetched, so
        // nothing is recorded as shown, and a later grant sees new messages.
        val foreground = ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
        if (foreground || !NotificationManagerCompat.from(context).areNotificationsEnabled()) {
            Log.i(TAG, "wake ignored foreground=$foreground")
            handled.countDown()
            return
        }
        val current = run
        if (current != null) {
            current.join(handled)
            return
        }
        Run(context).also {
            run = it
            it.join(handled)
            it.start()
        }
    }

    private class Run(private val context: Context) {
        private val startedAt = SystemClock.elapsedRealtime()
        private val notifier = WakeNotifier(context)
        private val waiting = mutableListOf<CountDownLatch>()
        private var engine: FlutterEngine? = null
        private var channel: MethodChannel? = null
        private var ready = false
        private var wakeQueued = true
        private var outstanding = 0
        private var answered = false
        private var genericShown = false
        private var finished = false
        private val genericCheck = Runnable {
            if (!answered) {
                log("generic_after_${GENERIC_AFTER_MS}ms")
                showGeneric()
            }
        }
        private val watchdog = Runnable { finish("watchdog") }

        private fun log(stage: String) =
            Log.i(TAG, "stage=$stage t=${SystemClock.elapsedRealtime() - startedAt}ms")

        fun join(latch: CountDownLatch) {
            waiting += latch
            // Wakes before Dart is ready are covered by its first pass.
            if (ready) requestWake()
        }

        fun start() {
            log("wake")
            notifier.createChannels()
            main.postDelayed(genericCheck, GENERIC_AFTER_MS)
            main.postDelayed(watchdog, WATCHDOG_MS)
            try {
                val engine = FlutterEngine(context).also { engine = it }
                log("engine_created")
                channel = MethodChannel(engine.dartExecutor.binaryMessenger, CHANNEL).also {
                    it.setMethodCallHandler { call, result ->
                        when (call.method) {
                            "ready" -> {
                                log("dart_ready")
                                ready = true
                                if (wakeQueued) {
                                    wakeQueued = false
                                    requestWake()
                                }
                                result.success(null)
                            }
                            else -> result.notImplemented()
                        }
                    }
                }
                engine.dartExecutor.executeDartEntrypoint(
                    DartExecutor.DartEntrypoint(
                        FlutterInjector.instance().flutterLoader().findAppBundlePath(),
                        ENTRYPOINT,
                    ),
                )
            } catch (error: Exception) {
                Log.e(TAG, "background engine failed to start", error)
                finish("engine_failed")
            }
        }

        private fun requestWake() {
            outstanding++
            val remaining = WATCHDOG_MS - RESULT_RESERVE_MS - (SystemClock.elapsedRealtime() - startedAt)
            channel!!.invokeMethod(
                "wake",
                mapOf("remainingMillis" to remaining.coerceAtLeast(0)),
                object : MethodChannel.Result {
                    override fun success(result: Any?) = answer { handleResult(result) }
                    override fun error(code: String, message: String?, details: Any?) =
                        answer {
                            Log.w(TAG, "dart wake failed: $code $message")
                            answered = true
                            showGeneric()
                        }
                    override fun notImplemented() = error("not_implemented", null, null)
                },
            )
        }

        private fun answer(handle: () -> Unit) {
            if (finished) return
            outstanding--
            try {
                handle()
            } catch (error: Exception) {
                // A result the handler can't read is a failed fetch, not a crash.
                Log.e(TAG, "unreadable wake result", error)
                answered = true
                if (!genericShown) showGeneric()
            }
            if (outstanding == 0) finish("done")
        }

        private fun handleResult(result: Any?) {
            val map = result as? Map<*, *> ?: throw IllegalArgumentException("Not a wake result")
            if (map["joined"] == true) return
            log("dart_result")
            val notifications = (map["notifications"] as List<*>).map { WakeNotification.fromDart(it) }
            answered = true
            when {
                notifications.isNotEmpty() -> {
                    notifier.postConversations(notifications, silent = genericShown)
                    log("posted count=${notifications.size}")
                    removeGeneric()
                }
                map["incomplete"] == true -> showGeneric()
                else -> removeGeneric()
            }
        }

        private fun showGeneric() {
            if (genericShown) return
            genericShown = true
            notifier.postGeneric()
        }

        /** Also clears the generic notification an earlier run left behind. */
        private fun removeGeneric() {
            genericShown = false
            notifier.cancelGeneric()
        }

        private fun finish(why: String) {
            if (finished) return
            finished = true
            main.removeCallbacks(genericCheck)
            main.removeCallbacks(watchdog)
            // No result at all (watchdog, failed engine): the wake must not be silent.
            if (!answered) showGeneric()
            engine?.destroy()
            engine = null
            run = null
            waiting.forEach { it.countDown() }
            log("finished_$why")
        }
    }
}
