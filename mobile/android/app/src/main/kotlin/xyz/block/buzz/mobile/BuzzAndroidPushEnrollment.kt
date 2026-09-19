package xyz.block.buzz.mobile

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import com.google.android.gms.tasks.Tasks
import com.google.firebase.appcheck.FirebaseAppCheck
import java.io.ByteArrayOutputStream
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import java.nio.charset.StandardCharsets
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.util.UUID
import java.util.concurrent.TimeUnit
import org.json.JSONObject

internal data class BuzzRelayOrigins(val push: String, val http: String)

internal fun buzzRelayOrigins(value: String): BuzzRelayOrigins {
    val parsed = URI(value)
    val scheme = parsed.scheme?.lowercase()
    require(
        scheme in setOf("ws", "wss") &&
            parsed.host != null &&
            parsed.userInfo == null &&
            parsed.query == null &&
            parsed.fragment == null &&
            (parsed.path.isNullOrEmpty() || parsed.path == "/")
    ) { "Invalid relay URL" }
    val push = URI(scheme, null, parsed.host, parsed.port, "", null, null).toString()
    val httpScheme = if (scheme == "wss") "https" else "http"
    val http = URI(httpScheme, null, parsed.host, parsed.port, "/", null, null).toString()
    return BuzzRelayOrigins(push, http)
}

internal class BuzzAndroidPushEnrollment(
    context: Context,
) {
    private val store = BuzzAndroidPushStore(context.applicationContext)

    fun endpointGrants(): List<BuzzAndroidPushGrant> = store.grants()

    fun enroll(
        fcmEndpoint: String,
        relayUrl: String,
        gatewayUrl: String,
    ): BuzzAndroidPushGrant = enroll(fcmEndpoint, relayUrl, gatewayUrl, true)

    private fun enroll(
        fcmEndpoint: String,
        relayUrl: String,
        gatewayUrl: String,
        allowStaleChallengeRecovery: Boolean,
    ): BuzzAndroidPushGrant {
        require(validEndpoint(fcmEndpoint)) { "Invalid FCM installation ID" }
        val relayOrigins = buzzRelayOrigins(relayUrl)
        val gatewayBase = gatewayBase(gatewayUrl)
        val relayKeys = fetchRelayKeys(relayOrigins.http, relayOrigins.push)
        val relayOrigin = relayKeys.origin
        val endpointHash = hex(sha256(fcmEndpoint.toByteArray(StandardCharsets.UTF_8)))
        val now = System.currentTimeMillis() / 1000
        val records = store.grants()
        records.firstOrNull {
            it.gatewayOrigin == gatewayBase &&
            it.relayOrigin == relayOrigin &&
                it.relayPubkey == relayKeys.pushPubkey &&
                it.endpointHash == endpointHash &&
                it.expiresAt > now + 300
        }?.let {
            store.removePending(relayOrigin)
            return if (it.relayMetadataPubkey == relayKeys.metadataPubkey) {
                it
            } else {
                it.copy(relayMetadataPubkey = relayKeys.metadataPubkey).also(store::saveGrant)
            }
        }
        records.firstOrNull {
            it.gatewayOrigin == gatewayBase &&
                it.relayPubkey == relayKeys.pushPubkey &&
                it.endpointHash == endpointHash &&
                it.expiresAt > now + 300
        }?.let { shared ->
            return shared.copy(
                relayOrigin = relayOrigin,
                relayMetadataPubkey = relayKeys.metadataPubkey,
                installationId = randomInstallationId(),
            ).also {
                store.saveGrant(it)
                store.removePending(relayOrigin)
            }
        }

        var pending = store.pending(relayOrigin)
        if (
            pending != null &&
                (pending.gatewayOrigin != gatewayBase ||
                    pending.relayPubkey != relayKeys.pushPubkey ||
                    pending.endpointHash != endpointHash ||
                    pending.expiresAt <= now)
        ) {
            store.removePending(relayOrigin)
            pending = null
        }
        if (pending == null) {
            val installationId = records.firstOrNull { it.relayOrigin == relayOrigin }
                ?.installationId ?: randomInstallationId()
            val reusable = records.firstOrNull {
                it.gatewayOrigin == gatewayBase &&
                    it.endpointHash == endpointHash &&
                    it.expiresAt > now
            }
            pending = if (reusable != null) {
                BuzzAndroidPendingEnrollment(
                    gatewayOrigin = gatewayBase,
                    relayOrigin = relayOrigin,
                    relayPubkey = relayKeys.pushPubkey,
                    endpointHash = endpointHash,
                    expiresAt = maxOf(reusable.expiresAt, now + INSTALLATION_LIFETIME_SECONDS),
                    installationId = installationId,
                    challengeId = null,
                    challenge = null,
                    gatewayInstallationHandle = reusable.gatewayInstallationHandle,
                    delegationGeneration = 0,
                )
            } else {
                val enrollmentChallenge = challenge(gatewayBase)
                BuzzAndroidPendingEnrollment(
                    gatewayOrigin = gatewayBase,
                    relayOrigin = relayOrigin,
                    relayPubkey = relayKeys.pushPubkey,
                    endpointHash = endpointHash,
                    expiresAt = Math.addExact(now, INSTALLATION_LIFETIME_SECONDS),
                    installationId = installationId,
                    challengeId = enrollmentChallenge.id,
                    challenge = enrollmentChallenge.value,
                    gatewayInstallationHandle = null,
                    delegationGeneration = 0,
                )
            }
            store.savePending(pending)
        }

        val keyPair = installationKey(endpointHash)
        var installationHandle = pending.gatewayInstallationHandle
        if (installationHandle == null) {
            val challengeId = requireNotNull(pending.challengeId)
            val challengeValue = requireNotNull(pending.challenge)
            val publicKey = publicKey(keyPair)
            val transcript = BuzzAndroidPushProtocol.enrollTranscript(
                challengeId = challengeId,
                challenge = challengeValue,
                publicKey = publicKey,
                endpoint = fcmEndpoint,
                expiresAt = pending.expiresAt,
            )
            val request = JSONObject()
                .put("v", 1)
                .put("challenge_id", challengeId)
                .put("challenge", challengeValue)
                .put("public_key", publicKey)
                .put("app_check_token", appCheckToken())
                .put("app_profile", BuzzAndroidPushProtocol.APP_PROFILE)
                .put("endpoint", fcmEndpoint)
                .put("endpoint_epoch", BuzzAndroidPushProtocol.ENDPOINT_EPOCH)
                .put("expires_at", pending.expiresAt)
                .put("assertion", sign(keyPair, transcript))
            val response = try {
                post(gatewayBase, "v1/installations/android", request, 201)
            } catch (error: GatewayHttpException) {
                if (error.status == 404 && allowStaleChallengeRecovery) {
                    store.removePending(relayOrigin)
                    return enroll(fcmEndpoint, relayUrl, gatewayUrl, false)
                }
                throw error
            }
            installationHandle = canonicalUuid(response.getString("installation_handle"))
            require(response.getLong("endpoint_epoch") == BuzzAndroidPushProtocol.ENDPOINT_EPOCH)
            require(response.getLong("expires_at") == pending.expiresAt)
            pending = pending.copy(gatewayInstallationHandle = installationHandle)
            store.savePending(pending)
        }

        val generationBase = maxOf(
            pending.delegationGeneration,
            records.filter {
                it.gatewayInstallationHandle == installationHandle &&
                    it.relayPubkey == relayKeys.pushPubkey
            }.maxOfOrNull(BuzzAndroidPushGrant::generation) ?: 0,
        )
        val generation = Math.addExact(generationBase, 1)
        pending = pending.copy(delegationGeneration = generation)
        store.savePending(pending)
        val delegationChallenge = challenge(gatewayBase)
        val notBefore = System.currentTimeMillis() / 1000
        val delegationTranscript = BuzzAndroidPushProtocol.delegateTranscript(
            challengeId = delegationChallenge.id,
            challenge = delegationChallenge.value,
            installationHandle = installationHandle,
            generation = generation,
            relayPubkey = relayKeys.pushPubkey,
            notBefore = notBefore,
            expiresAt = pending.expiresAt,
        )
        val delegation = post(
            gatewayBase,
            "v1/delegations",
            JSONObject()
                .put("v", 1)
                .put("challenge_id", delegationChallenge.id)
                .put("challenge", delegationChallenge.value)
                .put("installation_handle", installationHandle)
                .put("endpoint_epoch", BuzzAndroidPushProtocol.ENDPOINT_EPOCH)
                .put("generation", generation)
                .put("relay_pubkey", relayKeys.pushPubkey)
                .put("not_before", notBefore)
                .put("expires_at", pending.expiresAt)
                .put("assertion", sign(keyPair, delegationTranscript)),
            201,
        )
        val endpointGrant = delegation.getString("endpoint_grant")
        require(endpointGrant.isNotBlank() && endpointGrant.length <= 4096)
        return BuzzAndroidPushGrant(
            gatewayOrigin = gatewayBase,
            relayOrigin = relayOrigin,
            relayPubkey = relayKeys.pushPubkey,
            relayMetadataPubkey = relayKeys.metadataPubkey,
            gatewayInstallationHandle = installationHandle,
            installationId = pending.installationId,
            endpointGrant = endpointGrant,
            endpointHash = endpointHash,
            generation = generation,
            expiresAt = pending.expiresAt,
        ).also {
            store.saveGrant(it)
            store.removePending(relayOrigin)
        }
    }

    private fun appCheckToken(): String {
        val token = Tasks.await(
            FirebaseAppCheck.getInstance().getAppCheckToken(false),
            15,
            TimeUnit.SECONDS,
        ).token
        require(token.isNotBlank() && token.length <= 4096) { "Invalid Firebase App Check token" }
        return token
    }

    private fun fetchRelayKeys(httpOrigin: String, expectedPushOrigin: String): RelayKeys {
        val connection = URL(httpOrigin).openConnection() as HttpURLConnection
        connection.requestMethod = "GET"
        connection.setRequestProperty("Accept", "application/nostr+json")
        configure(connection)
        val body = response(connection, 200)
        val document = JSONObject(body)
        val push = document.getJSONObject("push")
        val advertisedOrigin = push.getString("origin")
        require(advertisedOrigin == expectedPushOrigin) {
            "Relay push origin does not match the configured relay"
        }
        val keys = push.getJSONArray("keys")
        val current = (0 until keys.length())
            .map(keys::getJSONObject)
            .filter { it.optBoolean("current", false) }
        require(current.size == 1) { "Relay has no unambiguous current push key" }
        val pubkey = current.single().getString("pubkey")
        require(PUBKEY.matches(pubkey)) { "Relay push key is invalid" }
        val metadata = document.optString("self").takeIf { PUBKEY.matches(it) }
        return RelayKeys(advertisedOrigin, pubkey, metadata)
    }

    private fun challenge(gatewayBase: String): Challenge {
        val response = post(
            gatewayBase,
            "v1/installations/challenges",
            JSONObject().put("v", 1),
            200,
        )
        val id = canonicalUuid(response.getString("challenge_id"))
        val value = response.getString("challenge")
        val decoded = Base64.decode(value, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
        require(decoded.size == 32) { "Gateway challenge is invalid" }
        require(response.getLong("expires_at") > System.currentTimeMillis() / 1000)
        return Challenge(id, value)
    }

    private fun post(base: String, route: String, body: JSONObject, expected: Int): JSONObject {
        val connection = URL(URL(base), route).openConnection() as HttpURLConnection
        connection.requestMethod = "POST"
        connection.doOutput = true
        connection.setRequestProperty("Content-Type", "application/json")
        configure(connection)
        val bytes = body.toString().toByteArray(StandardCharsets.UTF_8)
        require(bytes.size <= 23_896) { "Push gateway request is too large" }
        connection.setFixedLengthStreamingMode(bytes.size)
        connection.outputStream.use { it.write(bytes) }
        val status = connection.responseCode
        val response = readBounded(
            if (status in 200..299) connection.inputStream else connection.errorStream,
            64 * 1024,
        )
        if (status != expected) throw GatewayHttpException(status)
        return JSONObject(response)
    }

    private fun configure(connection: HttpURLConnection) {
        connection.connectTimeout = 10_000
        connection.readTimeout = 15_000
        connection.instanceFollowRedirects = false
        connection.useCaches = false
    }

    private fun response(connection: HttpURLConnection, expected: Int): String {
        val status = connection.responseCode
        val result = readBounded(
            if (status in 200..299) connection.inputStream else connection.errorStream,
            64 * 1024,
        )
        if (status != expected) throw GatewayHttpException(status)
        return result
    }

    private fun readBounded(stream: java.io.InputStream?, maximum: Int): String {
        if (stream == null) return ""
        stream.use { input ->
            val output = ByteArrayOutputStream()
            val buffer = ByteArray(4096)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                if (output.size() + count > maximum) throw IllegalArgumentException("Response too large")
                output.write(buffer, 0, count)
            }
            return output.toString(StandardCharsets.UTF_8.name())
        }
    }

    private fun installationKey(endpointHash: String): KeyPair {
        val alias = "buzz_push_android_${endpointHash.take(24)}"
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val existing = keyStore.getEntry(alias, null) as? KeyStore.PrivateKeyEntry
        if (existing != null) return KeyPair(existing.certificate.publicKey, existing.privateKey)
        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
        generator.initialize(
            KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN)
                .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
                .setDigests(KeyProperties.DIGEST_SHA256)
                .setUserAuthenticationRequired(false)
                .build(),
        )
        return generator.generateKeyPair()
    }

    private fun publicKey(keyPair: KeyPair): String {
        val publicKey = keyPair.public as ECPublicKey
        val x = unsigned32(publicKey.w.affineX.toByteArray())
        val y = unsigned32(publicKey.w.affineY.toByteArray())
        return Base64.encodeToString(byteArrayOf(4) + x + y, Base64.NO_WRAP)
    }

    private fun sign(keyPair: KeyPair, transcript: String): String {
        val signature = Signature.getInstance("SHA256withECDSA")
        signature.initSign(keyPair.private)
        signature.update(transcript.toByteArray(StandardCharsets.UTF_8))
        return Base64.encodeToString(signature.sign(), Base64.NO_WRAP)
    }

    private fun unsigned32(value: ByteArray): ByteArray {
        val first = value.indexOfFirst { it.toInt() != 0 }.let { if (it < 0) value.size else it }
        val unsigned = value.copyOfRange(first, value.size)
        require(unsigned.size <= 32)
        return ByteArray(32 - unsigned.size) + unsigned
    }

    private fun gatewayBase(value: String): String {
        val parsed = URI(value)
        require(
            parsed.host != null &&
                parsed.userInfo == null &&
                parsed.query == null &&
                parsed.fragment == null &&
                (parsed.path.isNullOrEmpty() || parsed.path == "/")
        )
        require(parsed.scheme == "https" || (parsed.scheme == "http" && parsed.host in LOOPBACK_HOSTS))
        return URI(parsed.scheme, null, parsed.host, parsed.port, "/", null, null).toString()
    }

    private fun canonicalUuid(value: String): String {
        val uuid = UUID.fromString(value).toString()
        require(uuid == value) { "Gateway UUID is not canonical" }
        return uuid
    }

    private fun randomInstallationId(): String = ByteArray(16)
        .also(SecureRandom()::nextBytes)
        .let(::hex)

    private fun validEndpoint(value: String): Boolean =
        value.isNotEmpty() && value.length <= 4096 && value.all {
            it.isLetterOrDigit() || it == '-' || it == '_' || it == ':' || it == '.'
        }

    private fun sha256(value: ByteArray): ByteArray = MessageDigest.getInstance("SHA-256").digest(value)

    private fun hex(value: ByteArray): String = value.joinToString("") { "%02x".format(it) }

    private data class Challenge(val id: String, val value: String)
    private data class RelayKeys(
        val origin: String,
        val pushPubkey: String,
        val metadataPubkey: String?,
    )
    private class GatewayHttpException(val status: Int) : Exception("Push gateway rejected request")

    companion object {
        private const val INSTALLATION_LIFETIME_SECONDS =
            BuzzAndroidPushProtocol.INSTALLATION_LIFETIME_SECONDS
        private val PUBKEY = Regex("^[0-9a-f]{64}$")
        private val LOOPBACK_HOSTS = setOf("localhost", "127.0.0.1", "::1")
    }
}
