package xyz.block.buzz.mobile

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Notification
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Debug
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.os.SystemClock
import android.util.Log
import io.flutter.FlutterInjector
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.embedding.engine.dart.DartExecutor
import io.flutter.plugin.common.MethodChannel

/**
 * PROTOTYPE: stands in for the FCM wake. Posts a placeholder notification, runs
 * a headless Dart engine that fetches the newest relay message, and updates the
 * notification. All stage times are SystemClock.elapsedRealtime, logged under
 * the PUSHPROTO tag.
 */
class PushProtoReceiver : BroadcastReceiver() {
    companion object {
        const val TAG = "PUSHPROTO"
        const val ACTION_WAKE = "xyz.block.buzz.mobile.PROTO_WAKE"
        const val ACTION_SEED = "xyz.block.buzz.mobile.PROTO_SEED"
        const val ACTION_KILL = "xyz.block.buzz.mobile.PROTO_KILL"
        const val CHANNEL_ID = "proto_push"
        const val NOTIFICATION_ID = 4242
        const val WATCHDOG_MS = 9_500L
        const val ENTRY_LIB = "package:buzz/push_proto/push_wake_entry.dart"
    }

    override fun onReceive(context: Context, intent: Intent) {
        val tWake = SystemClock.elapsedRealtime()
        when (intent.action) {
            ACTION_KILL -> Process.killProcess(Process.myPid())
            ACTION_WAKE, ACTION_SEED -> run(context.applicationContext, intent, tWake)
        }
    }

    private fun log(name: String, t: Long = SystemClock.elapsedRealtime()) {
        Log.i(TAG, "stage $name t=$t")
    }

    private fun run(context: Context, intent: Intent, tWake: Long) {
        val seeding = intent.action == ACTION_SEED
        val pending = goAsync()
        Log.i(TAG, "begin pid=${Process.myPid()} procStart=${Process.getStartElapsedRealtime()}")
        log("wake", tWake)
        val manager = context.getSystemService(NotificationManager::class.java)
        if (!seeding) postPlaceholder(context, manager)
        if (!seeding) log("placeholder_posted")

        val engine = FlutterEngine(context)
        log("engine_created")

        var finished = false
        val handler = Handler(Looper.getMainLooper())
        val hold = intent.getLongExtra("hold", 0L)
        fun finish(why: String) {
            if (finished) return
            finished = true
            handler.removeCallbacksAndMessages(null)
            val pss = Debug.MemoryInfo().also { Debug.getMemoryInfo(it) }.totalPss
            Log.i(TAG, "finish $why pssKb=$pss nativeHeapKb=${Debug.getNativeHeapAllocatedSize() / 1024}")
            log("handler_finished")
            engine.destroy()
            pending.finish()
        }
        handler.postDelayed({ finish("watchdog_timeout") }, WATCHDOG_MS)

        MethodChannel(engine.dartExecutor.binaryMessenger, "buzz/push_proto")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "stage" -> {
                        val args = call.arguments as Map<*, *>
                        log(args["name"] as String, (args["t"] as Number).toLong())
                    }
                    "done" -> {
                        val args = call.arguments as Map<*, *>
                        postUpdated(context, manager, args["title"] as String, args["body"] as String)
                        log("notification_updated")
                        // PROTOTYPE: keep the engine alive so memory can be sampled.
                        if (hold > 0) handler.postDelayed({ finish("ok") }, hold) else finish("ok")
                    }
                    "seeded" -> finish("seeded")
                    "fail" -> {
                        Log.w(TAG, "dart failed: ${call.arguments}")
                        finish("dart_failed")
                    }
                }
                result.success(null)
            }

        val offset = SystemClock.elapsedRealtime() - System.currentTimeMillis()
        val args = listOf(
            offset.toString(),
            if (seeding) "seed" else "wake",
            intent.getStringExtra("relay") ?: "",
            intent.getStringExtra("nsec") ?: "",
        )
        val bundle = FlutterInjector.instance().flutterLoader().findAppBundlePath()
        engine.dartExecutor.executeDartEntrypoint(
            DartExecutor.DartEntrypoint(bundle, ENTRY_LIB, "pushWakeMain"),
            args,
        )
        log("dart_started")
    }

    private fun builder(context: Context, manager: NotificationManager): Notification.Builder {
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "Proto push", NotificationManager.IMPORTANCE_HIGH),
        )
        return Notification.Builder(context, CHANNEL_ID).setSmallIcon(R.mipmap.ic_launcher)
    }

    private fun postPlaceholder(context: Context, manager: NotificationManager) {
        manager.notify(
            NOTIFICATION_ID,
            builder(context, manager).setContentTitle("Buzz").setContentText("New message").build(),
        )
    }

    private fun postUpdated(context: Context, manager: NotificationManager, title: String, body: String) {
        manager.notify(
            NOTIFICATION_ID,
            builder(context, manager).setContentTitle(title).setContentText(body)
                .setOnlyAlertOnce(true).build(),
        )
    }
}
