package xyz.block.buzz.mobile

internal object BuzzAndroidPushProtocol {
    const val APP_PROFILE = "buzz-android-fcm"
    const val ENDPOINT_EPOCH = 1L
    const val INSTALLATION_LIFETIME_SECONDS = 2_592_000L

    fun enrollTranscript(
        challengeId: String,
        challenge: String,
        publicKey: String,
        endpoint: String,
        expiresAt: Long,
    ): String = "buzz.push.enroll-android.v1\n" + jsonObject(
        "v" to "1",
        "audience" to quote("https://push.buzz.xyz/v1/installations/android"),
        "challenge_id" to quote(challengeId),
        "challenge" to quote(challenge),
        "public_key" to quote(publicKey),
        "app_profile" to quote(APP_PROFILE),
        "endpoint" to quote(endpoint),
        "endpoint_epoch" to ENDPOINT_EPOCH.toString(),
        "expires_at" to expiresAt.toString(),
    )

    fun delegateTranscript(
        challengeId: String,
        challenge: String,
        installationHandle: String,
        generation: Long,
        relayPubkey: String,
        notBefore: Long,
        expiresAt: Long,
    ): String = "buzz.push.delegate.v1\n" + jsonObject(
        "v" to "1",
        "audience" to quote("https://push.buzz.xyz/v1/delegations"),
        "challenge_id" to quote(challengeId),
        "challenge" to quote(challenge),
        "installation_handle" to quote(installationHandle),
        "endpoint_epoch" to ENDPOINT_EPOCH.toString(),
        "generation" to generation.toString(),
        "relay_pubkey" to quote(relayPubkey),
        "not_before" to notBefore.toString(),
        "expires_at" to expiresAt.toString(),
    )

    fun quote(value: String): String {
        val result = StringBuilder(value.length + 2).append('"')
        value.forEach { character ->
            when (character) {
                '"' -> result.append("\\\"")
                '\\' -> result.append("\\\\")
                '\b' -> result.append("\\b")
                '\u000C' -> result.append("\\f")
                '\n' -> result.append("\\n")
                '\r' -> result.append("\\r")
                '\t' -> result.append("\\t")
                else -> if (character.code < 0x20) {
                    result.append("\\u%04x".format(character.code))
                } else {
                    result.append(character)
                }
            }
        }
        return result.append('"').toString()
    }

    private fun jsonObject(vararg fields: Pair<String, String>): String = fields.joinToString(
        separator = ",",
        prefix = "{",
        postfix = "}",
    ) { (name, value) -> "${quote(name)}:$value" }
}
