package xyz.block.buzz.mobile

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

internal data class BuzzAndroidPushGrant(
    val gatewayOrigin: String,
    val relayOrigin: String,
    val relayPubkey: String,
    val relayMetadataPubkey: String?,
    val gatewayInstallationHandle: String,
    val installationId: String,
    val endpointGrant: String,
    val endpointHash: String,
    val generation: Long,
    val expiresAt: Long,
) {
    fun flutterArguments(): Map<String, Any> = buildMap {
        put("relayOrigin", relayOrigin)
        put("relayPubkey", relayPubkey)
        put("installationId", installationId)
        put("endpointGrant", endpointGrant)
        put("endpointHash", endpointHash)
        put("appProfile", BuzzAndroidPushProtocol.APP_PROFILE)
        put("endpointEpoch", BuzzAndroidPushProtocol.ENDPOINT_EPOCH)
        put("generation", generation)
        put("expiresAt", expiresAt)
    }

    fun json(): JSONObject = JSONObject()
        .put("gatewayOrigin", gatewayOrigin)
        .put("relayOrigin", relayOrigin)
        .put("relayPubkey", relayPubkey)
        .put("relayMetadataPubkey", relayMetadataPubkey)
        .put("gatewayInstallationHandle", gatewayInstallationHandle)
        .put("installationId", installationId)
        .put("endpointGrant", endpointGrant)
        .put("endpointHash", endpointHash)
        .put("generation", generation)
        .put("expiresAt", expiresAt)

    companion object {
        fun fromJson(json: JSONObject): BuzzAndroidPushGrant = BuzzAndroidPushGrant(
            gatewayOrigin = json.getString("gatewayOrigin"),
            relayOrigin = json.getString("relayOrigin"),
            relayPubkey = json.getString("relayPubkey"),
            relayMetadataPubkey = json.optString("relayMetadataPubkey").takeIf(String::isNotBlank),
            gatewayInstallationHandle = json.getString("gatewayInstallationHandle"),
            installationId = json.getString("installationId"),
            endpointGrant = json.getString("endpointGrant"),
            endpointHash = json.getString("endpointHash"),
            generation = json.getLong("generation"),
            expiresAt = json.getLong("expiresAt"),
        )
    }
}

internal data class BuzzAndroidPendingEnrollment(
    val gatewayOrigin: String,
    val relayOrigin: String,
    val relayPubkey: String,
    val endpointHash: String,
    val expiresAt: Long,
    val installationId: String,
    val challengeId: String?,
    val challenge: String?,
    val gatewayInstallationHandle: String?,
    val delegationGeneration: Long,
) {
    fun json(): JSONObject = JSONObject()
        .put("gatewayOrigin", gatewayOrigin)
        .put("relayOrigin", relayOrigin)
        .put("relayPubkey", relayPubkey)
        .put("endpointHash", endpointHash)
        .put("expiresAt", expiresAt)
        .put("installationId", installationId)
        .put("challengeId", challengeId)
        .put("challenge", challenge)
        .put("gatewayInstallationHandle", gatewayInstallationHandle)
        .put("delegationGeneration", delegationGeneration)

    companion object {
        fun fromJson(json: JSONObject): BuzzAndroidPendingEnrollment =
            BuzzAndroidPendingEnrollment(
                gatewayOrigin = json.getString("gatewayOrigin"),
                relayOrigin = json.getString("relayOrigin"),
                relayPubkey = json.getString("relayPubkey"),
                endpointHash = json.getString("endpointHash"),
                expiresAt = json.getLong("expiresAt"),
                installationId = json.getString("installationId"),
                challengeId = json.optString("challengeId").takeIf(String::isNotBlank),
                challenge = json.optString("challenge").takeIf(String::isNotBlank),
                gatewayInstallationHandle = json.optString("gatewayInstallationHandle")
                    .takeIf(String::isNotBlank),
                delegationGeneration = json.optLong("delegationGeneration", 0),
            )
    }
}

internal class BuzzAndroidPushStore(context: Context) {
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)

    fun fcmEndpoint(): String? = preferences.getString(FCM_ENDPOINT, null)?.takeIf(String::isNotBlank)

    fun saveFcmEndpoint(endpoint: String) {
        check(preferences.edit().putString(FCM_ENDPOINT, endpoint).commit()) {
            "Unable to persist the FCM installation ID"
        }
    }

    fun grants(): List<BuzzAndroidPushGrant> {
        val array = JSONArray(preferences.getString(GRANTS, "[]") ?: "[]")
        return List(array.length()) { index ->
            BuzzAndroidPushGrant.fromJson(array.getJSONObject(index))
        }
    }

    fun saveGrant(grant: BuzzAndroidPushGrant) {
        // One relay origin owns one current installation record. FCM token
        // rotation changes the endpoint hash but must replace, rather than
        // retain, the stale relay-facing grant.
        val updated = grants().filterNot { it.relayOrigin == grant.relayOrigin } + grant
        persistGrants(updated)
    }

    fun pending(relayOrigin: String): BuzzAndroidPendingEnrollment? {
        val all = JSONObject(preferences.getString(PENDING, "{}") ?: "{}")
        return all.optJSONObject(relayOrigin)?.let(BuzzAndroidPendingEnrollment::fromJson)
    }

    fun savePending(pending: BuzzAndroidPendingEnrollment) {
        val all = JSONObject(preferences.getString(PENDING, "{}") ?: "{}")
        all.put(pending.relayOrigin, pending.json())
        check(preferences.edit().putString(PENDING, all.toString()).commit()) {
            "Unable to persist pending Android push enrollment"
        }
    }

    fun removePending(relayOrigin: String) {
        val all = JSONObject(preferences.getString(PENDING, "{}") ?: "{}")
        all.remove(relayOrigin)
        check(preferences.edit().putString(PENDING, all.toString()).commit()) {
            "Unable to clear pending Android push enrollment"
        }
    }

    private fun persistGrants(grants: List<BuzzAndroidPushGrant>) {
        val array = JSONArray()
        grants.forEach { array.put(it.json()) }
        check(preferences.edit().putString(GRANTS, array.toString()).commit()) {
            "Unable to persist Android push grants"
        }
    }

    companion object {
        private const val PREFERENCES = "buzz_android_push_v1"
        private const val FCM_ENDPOINT = "fcm_endpoint"
        private const val GRANTS = "endpoint_grants"
        private const val PENDING = "pending_enrollments"
    }
}
