package xyz.block.buzz.mobile

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class BuzzAndroidPushProtocolTest {
    @Test
    fun relayOriginsKeepWebSocketIdentityAndUseHttpForDiscovery() {
        assertEquals(
            BuzzRelayOrigins(
                push = "wss://relay.example:8443",
                http = "https://relay.example:8443/",
            ),
            buzzRelayOrigins("wss://relay.example:8443/"),
        )
        assertEquals(
            BuzzRelayOrigins(push = "ws://localhost:3000", http = "http://localhost:3000/"),
            buzzRelayOrigins("ws://localhost:3000"),
        )
    }

    @Test
    fun relayOriginsRejectNonOriginUrls() {
        for (value in listOf("https://relay.example", "wss://relay.example/path", "not-a-url")) {
            assertFailsWith<IllegalArgumentException> { buzzRelayOrigins(value) }
        }
    }

    @Test
    fun enrollmentTranscriptMatchesGatewayCanonicalFieldOrder() {
        assertEquals(
            "buzz.push.enroll-android.v1\n" +
                "{\"v\":1,\"audience\":\"https://push.buzz.xyz/v1/installations/android\"," +
                "\"challenge_id\":\"00000000-0000-0000-0000-000000000000\"," +
                "\"challenge\":\"abc\",\"public_key\":\"key\"," +
                "\"app_profile\":\"buzz-android-fcm\",\"endpoint\":\"token:one\"," +
                "\"endpoint_epoch\":1,\"expires_at\":99}",
            BuzzAndroidPushProtocol.enrollTranscript(
                challengeId = "00000000-0000-0000-0000-000000000000",
                challenge = "abc",
                publicKey = "key",
                endpoint = "token:one",
                expiresAt = 99,
            ),
        )
    }

    @Test
    fun delegationTranscriptMatchesGatewayCanonicalFieldOrder() {
        assertEquals(
            "buzz.push.delegate.v1\n" +
                "{\"v\":1,\"audience\":\"https://push.buzz.xyz/v1/delegations\"," +
                "\"challenge_id\":\"00000000-0000-0000-0000-000000000000\"," +
                "\"challenge\":\"abc\",\"installation_handle\":" +
                "\"11111111-1111-4111-8111-111111111111\"," +
                "\"endpoint_epoch\":1,\"generation\":2,\"relay_pubkey\":\"aaaa\"," +
                "\"not_before\":90,\"expires_at\":99}",
            BuzzAndroidPushProtocol.delegateTranscript(
                challengeId = "00000000-0000-0000-0000-000000000000",
                challenge = "abc",
                installationHandle = "11111111-1111-4111-8111-111111111111",
                generation = 2,
                relayPubkey = "aaaa",
                notBefore = 90,
                expiresAt = 99,
            ),
        )
    }

    @Test
    fun jsonStringsAreEscapedBeforeSigning() {
        assertEquals("\"line\\n\\\"quoted\\\"\\\\\"", BuzzAndroidPushProtocol.quote("line\n\"quoted\"\\"))
    }
}
