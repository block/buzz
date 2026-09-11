import CryptoKit
import Foundation
import XCTest

@testable import BuzzPushKit

/// Replays the gateway-generated known-answer vectors
/// (`crates/buzz-push-gateway/tests/vectors/app_attest_transcripts.json`)
/// against the Swift canonical encoder. Byte-for-byte transcript equality and
/// SHA-256 equality are both asserted, so a drift on either side breaks a test
/// instead of silently stranding iOS clients with `401 invalid_attestation`.
final class BuzzPushTranscriptTests: XCTestCase {
    // MARK: Fixture

    struct Fixture: Decodable {
        struct Vector: Decodable {
            let name: String
            let domain: String
            let transcript: String
            let sha256: String
        }

        let vectors: [Vector]
    }

    static func fixture(
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws -> Fixture {
        let path = try XCTUnwrap(
            Bundle.module.url(
                forResource: "app_attest_transcripts",
                withExtension: "json"
            ),
            "missing bundled gateway transcript fixture app_attest_transcripts.json in \(Bundle.module.bundleURL.path)",
            file: file,
            line: line
        )
        let data = try Data(contentsOf: path)
        return try JSONDecoder().decode(Fixture.self, from: data)
    }

    // Deterministic inputs mirroring the fixture's `inputs` block.
    static let challengeId = UUID(uuidString: "11111111-1111-4111-8111-111111111111")!
    static let installationHandle = UUID(uuidString: "22222222-2222-4222-8222-222222222222")!
    static let challenge = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
    static let keyId = "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqo="
    static let appProfile = "buzz-ios-dogfood"
    static let endpoint = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"
    static let relayPubkey = String(repeating: "a", count: 64)
    static let notBefore: Int64 = 1_752_620_000
    static let expiresAt: Int64 = 1_752_624_000

    private func assertMatchesVector(_ name: String, _ bytes: Data,
                                     file: StaticString = #filePath, line: UInt = #line) throws {
        guard let vector = try Self.fixture(file: file, line: line).vectors.first(where: { $0.name == name }) else {
            XCTFail("missing fixture vector \(name)", file: file, line: line)
            return
        }
        XCTAssertEqual(String(decoding: bytes, as: UTF8.self), vector.transcript,
                       "\(name) transcript bytes drifted from gateway ground truth",
                       file: file, line: line)
        let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
        XCTAssertEqual(digest, vector.sha256,
                       "\(name) sha256 drifted from gateway ground truth",
                       file: file, line: line)
    }

    // MARK: Known-answer vectors

    func testEnrollVector() throws {
        try assertMatchesVector("enroll", BuzzPushTranscript.enroll(
            challengeId: Self.challengeId,
            challenge: Self.challenge,
            keyId: Self.keyId,
            appProfile: Self.appProfile,
            endpoint: Self.endpoint,
            endpointEpoch: 1,
            expiresAt: Self.expiresAt
        ))
    }

    func testDelegateVector() throws {
        try assertMatchesVector("delegate", BuzzPushTranscript.delegate(
            challengeId: Self.challengeId,
            challenge: Self.challenge,
            installationHandle: Self.installationHandle,
            endpointEpoch: 1,
            generation: 1,
            relayPubkey: Self.relayPubkey,
            notBefore: Self.notBefore,
            expiresAt: Self.expiresAt
        ))
    }

    func testRotateEndpointVector() throws {
        try assertMatchesVector("rotate_endpoint", BuzzPushTranscript.rotateEndpoint(
            challengeId: Self.challengeId,
            challenge: Self.challenge,
            installationHandle: Self.installationHandle,
            endpointEpoch: 1,
            newEndpointEpoch: 2,
            endpoint: Self.endpoint
        ))
    }

    func testRevokeDelegationVector() throws {
        try assertMatchesVector("revoke_delegation", BuzzPushTranscript.revokeDelegation(
            challengeId: Self.challengeId,
            challenge: Self.challenge,
            installationHandle: Self.installationHandle,
            relayPubkey: Self.relayPubkey,
            generation: 2
        ))
    }

    func testRevokeInstallationVector() throws {
        try assertMatchesVector("revoke_installation", BuzzPushTranscript.revokeInstallation(
            challengeId: Self.challengeId,
            challenge: Self.challenge,
            installationHandle: Self.installationHandle,
            endpointEpoch: 1,
            newEndpointEpoch: 2
        ))
    }

    func testAllFixtureVectorsCovered() throws {
        XCTAssertEqual(
            Set(try Self.fixture().vectors.map(\.name)),
            ["enroll", "delegate", "rotate_endpoint", "revoke_delegation", "revoke_installation"],
            "fixture gained or lost a vector; add/remove the matching known-answer test"
        )
    }

    // MARK: Escaping edges (the exact JSONSerialization failure modes)

    func testSolidusIsNotEscaped() throws {
        // The whole reason this encoder exists: '/' must pass through raw.
        XCTAssertEqual(try BuzzPushTranscript.CanonicalObject.escape("https://push.buzz.xyz/v1", field: "audience"),
                       "https://push.buzz.xyz/v1")
    }

    func testMinimalEscaping() throws {
        XCTAssertEqual(try BuzzPushTranscript.CanonicalObject.escape("a\"b\\c\u{08}\u{09}\u{0A}\u{0C}\u{0D}\u{01}", field: "x"),
                       "a\\\"b\\\\c\\b\\t\\n\\f\\r\\u0001")
    }

    func testNonASCIIRejected() {
        XCTAssertThrowsError(try BuzzPushTranscript.CanonicalObject.escape("caf\u{00E9}", field: "app_profile")) {
            XCTAssertEqual($0 as? BuzzPushTranscriptError, .nonASCIIInput(field: "app_profile"))
        }
    }

    func testUUIDLowercased() throws {
        var o = BuzzPushTranscript.CanonicalObject()
        o.uuid("k", UUID(uuidString: "ABCDEF12-3456-4789-8ABC-DEF123456789")!)
        XCTAssertEqual(o.encoded(), "{\"k\":\"abcdef12-3456-4789-8abc-def123456789\"}")
    }
}
