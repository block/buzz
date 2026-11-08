package xyz.block.buzz.mobile

import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel

/** One browser session. Android delivers onNewIntent before onResume on return. */
internal class BrowserAuthPlugin(
    activity: Activity,
    messenger: BinaryMessenger,
    private val launch: (Uri) -> Unit = { uri ->
        CustomTabsIntent.Builder().build().launchUrl(activity, uri)
    },
) {
    private val channel = MethodChannel(messenger, "buzz/browser_auth")
    private var pending: MethodChannel.Result? = null
    private var state: String? = null
    private var departed = false

    init {
        channel.setMethodCallHandler { call, result ->
            when (call.method) {
                "start" -> {
                    val url = call.argument<String>("url")?.let(Uri::parse)
                    if (url == null || url.scheme != "https" || url.host.isNullOrEmpty()) {
                        result.error("unavailable", "Unable to open sign-in.", null)
                    } else {
                        cancel()
                        pending = result
                        state = url.getQueryParameter("state")
                        departed = false
                        try {
                            launch(url)
                        } catch (_: Exception) {
                            finishError("unavailable")
                        }
                    }
                }
                "cancel" -> {
                    cancel()
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        }
    }

    fun onPause() {
        if (pending != null) departed = true
    }

    fun onResume() {
        if (departed) cancel()
    }

    fun onNewIntent(intent: Intent) {
        val uri = intent.data ?: return
        if (pending == null || uri.scheme != "xyz.block.buzz.mobile" ||
            uri.encodedSchemeSpecificPart.substringBefore('?') != "/auth/callback"
        ) return
        // A rejected auth callback also brings the Activity back. Its resume
        // is not a browser dismissal and must not cancel the newer attempt.
        departed = false
        if (uri.getQueryParameter("state") != state) return
        val result = pending
        clear()
        // Dart remains authoritative for canonical URI, state, expiry and PKCE.
        result?.success(uri.toString())
    }

    private fun clear() {
        pending = null
        state = null
        departed = false
    }

    private fun finishError(code: String) {
        val result = pending
        clear()
        result?.error(code, "Sign-in did not complete.", null)
    }

    private fun cancel() = finishError("cancelled")

    fun dispose() {
        cancel()
        channel.setMethodCallHandler(null)
    }
}
