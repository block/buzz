package xyz.block.buzz.mobile

import android.app.Activity
import android.os.Bundle

/**
 * Debug-only entry point for previewing the Android system Live Update.
 *
 * Launch the exported activity explicitly from a development device after
 * granting notification permission. Release builds do not include it.
 */
class AgentLiveUpdateDemoActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        AgentLiveUpdateManager(this).show(
            AgentLiveUpdatePayload(
                title = intent.getStringExtra(EXTRA_TITLE) ?: "Codex is working",
                body = intent.getStringExtra(EXTRA_BODY) ?: "In #agents",
                activeCount = intent.getIntExtra(EXTRA_ACTIVE_COUNT, 1),
                startedAtMillis = System.currentTimeMillis() - DEMO_ELAPSED_MILLIS,
                expiresAtMillis = System.currentTimeMillis() + DEMO_LIFETIME_MILLIS,
                channelId = "agents-demo",
                messageId = null,
            ),
        )
        finish()
    }

    companion object {
        private const val EXTRA_TITLE = "title"
        private const val EXTRA_BODY = "body"
        private const val EXTRA_ACTIVE_COUNT = "activeCount"
        private const val DEMO_ELAPSED_MILLIS = 32_000L
        private const val DEMO_LIFETIME_MILLIS = 8 * 60 * 60_000L
    }
}
