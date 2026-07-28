package xyz.block.buzz.mobile

internal data class AgentLiveUpdatePayload(
    val title: String,
    val body: String,
    val activeCount: Int,
    val startedAtMillis: Long,
    val expiresAtMillis: Long,
    val channelId: String,
    val messageId: String?,
) {
    companion object {
        fun from(arguments: Any?): AgentLiveUpdatePayload? {
            val values = arguments as? Map<*, *> ?: return null
            val title = (values["title"] as? String)?.takeIf { it.isNotBlank() } ?: return null
            val body = (values["body"] as? String)?.takeIf { it.isNotBlank() } ?: return null
            val channelId =
                (values["channelId"] as? String)?.takeIf { it.isNotBlank() } ?: return null
            val activeCount = (values["activeCount"] as? Number)?.toInt() ?: return null
            val startedAtMillis =
                (values["startedAtMillis"] as? Number)?.toLong() ?: return null
            val expiresAtMillis =
                (values["expiresAtMillis"] as? Number)?.toLong() ?: return null

            if (activeCount < 1 || startedAtMillis < 0 || expiresAtMillis <= startedAtMillis) {
                return null
            }

            return AgentLiveUpdatePayload(
                title = title,
                body = body,
                activeCount = activeCount,
                startedAtMillis = startedAtMillis,
                expiresAtMillis = expiresAtMillis,
                channelId = channelId,
                messageId = (values["messageId"] as? String)?.takeIf { it.isNotBlank() },
            )
        }
    }
}
