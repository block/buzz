import CryptoKit
import Foundation
import Security
import XCTest

@testable import BuzzPushKit

#if canImport(FoundationNetworking)
  import FoundationNetworking
#endif

final class BuzzDevPushEnrollmentDriverTests: XCTestCase {
  private static let gatewayURL = URL(string: "http://push.example/")!
  private static let gatewayOrigin = "http://push.example"
  private static let relayURL = URL(string: "wss://relay.example/")!
  private static let relayPubkey = String(repeating: "a", count: 64)
  private static let firstChallengeId = "11111111-1111-4111-8111-111111111111"
  private static let secondChallengeId = "33333333-3333-4333-8333-333333333333"
  private static let installationHandle = "22222222-2222-4222-8222-222222222222"
  private static let installationId = "000102030405060708090a0b0c0d0e0f"
  private static let challenge = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
  private static let now: Int64 = 1_752_620_000
  private static let expiresAt: Int64 = 1_752_624_000
  private static let endpoint =
    "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"
  fileprivate static let keyId = "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqo="
  fileprivate static let attestation = Data("test-attestation".utf8).base64EncodedString()
  fileprivate static let assertion = Data("buzz-dev-app-assertion-v1".utf8).base64EncodedString()

  override func setUp() {
    super.setUp()
    URLProtocolStub.reset()
  }

  override func tearDown() {
    URLProtocolStub.reset()
    super.tearDown()
  }

  func testEnrollmentPinsTranscriptsAndPersistsOpaqueGrant() async throws {
    let store = MemoryGrantStore()
    let appAttest = RecordingAppAttest()
    let driver = try makeDriver(store: store, appAttest: appAttest)
    var challengeCount = 0
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        XCTAssertEqual(request.value(forHTTPHeaderField: "Accept"), "application/nostr+json")
        return Self.response(
          request,
          status: 200,
          json: [
            "self": Self.relayPubkey,
            "push": [
              "keys": [
                ["id": "current", "pubkey": Self.relayPubkey, "current": true]
              ]
            ],
          ]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        challengeCount += 1
        let body = try Self.body(request)
        XCTAssertEqual(body["v"] as? Int, 1)
        XCTAssertEqual(body.count, 1)
        let id = challengeCount == 1 ? Self.firstChallengeId : Self.secondChallengeId
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": id,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/installations"):
        let body = try Self.body(request)
        XCTAssertEqual(body["endpoint"] as? String, Self.endpoint)
        XCTAssertEqual(body["endpoint_epoch"] as? Int, 1)
        XCTAssertEqual(body["expires_at"] as? Int64, Self.expiresAt)
        XCTAssertEqual(body["challenge_id"] as? String, Self.firstChallengeId)
        XCTAssertEqual(body["challenge"] as? String, Self.challenge)
        XCTAssertEqual(body["key_id"] as? String, Self.keyId)
        XCTAssertEqual(body["attestation"] as? String, Self.attestation)
        XCTAssertEqual(body["app_profile"] as? String, "buzz-ios-dogfood")
        return Self.response(
          request,
          status: 201,
          json: [
            "installation_handle": Self.installationHandle,
            "endpoint_epoch": 1,
            "expires_at": Self.expiresAt,
          ]
        )
      case ("POST", "http://push.example/v1/delegations"):
        let body = try Self.body(request)
        XCTAssertEqual(body["relay_pubkey"] as? String, Self.relayPubkey)
        XCTAssertEqual(body["installation_handle"] as? String, Self.installationHandle)
        XCTAssertEqual(body["challenge_id"] as? String, Self.secondChallengeId)
        XCTAssertEqual(body["challenge"] as? String, Self.challenge)
        XCTAssertEqual(body["endpoint_epoch"] as? Int, 1)
        XCTAssertEqual(body["not_before"] as? Int64, Self.now)
        XCTAssertEqual(body["expires_at"] as? Int64, Self.expiresAt)
        XCTAssertEqual(body["assertion"] as? String, Self.assertion)
        XCTAssertEqual(body["generation"] as? Int, 1)
        return Self.response(
          request,
          status: 201,
          json: ["endpoint_grant": "opaque-grant"]
        )
      default:
        XCTFail(
          "Unexpected request \(request.httpMethod ?? "nil") \(request.url?.absoluteString ?? "nil")"
        )
        return Self.response(request, status: 500, json: [:])
      }
    }

    let record = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: Self.relayURL
    )

    XCTAssertEqual(appAttest.clientData.count, 2)
    XCTAssertEqual(record.gatewayOrigin, Self.gatewayOrigin)
    XCTAssertEqual(record.relayOrigin, "wss://relay.example")
    try assertMatchesVector(
      "enroll",
      actual: appAttest.clientData[0],
      expectedSHA256: "58274bd9e9a86489fe5bae36aecbe89618824433189405ff4de8b18b58384270",
      fixture: makeFixtureTranscript(
        name: "enroll",
        replacements: []
      )
    )
    try assertMatchesVector(
      "delegate",
      actual: appAttest.clientData[1],
      expectedSHA256: "f186db11cb53e4e80f09489c11dd18afc9b641683c3d72a67113c57d32fca323",
      fixture: makeFixtureTranscript(
        name: "delegate",
        replacements: [
          (Self.firstChallengeId, Self.secondChallengeId)
        ]
      )
    )
    XCTAssertEqual(
      record,
      BuzzPushEndpointGrantRecord(
        gatewayOrigin: Self.gatewayOrigin,
        relayOrigin: "wss://relay.example",
        relayPubkey: Self.relayPubkey,
        relayMetadataPubkey: Self.relayPubkey,
        gatewayInstallationHandle: Self.installationHandle,
        appAttestKeyId: Self.keyId,
        installationId: Self.installationId,
        endpointGrant: "opaque-grant",
        endpointHash: Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init)))),
        appProfile: "buzz-ios-dogfood",
        endpointEpoch: 1,
        generation: 1,
        expiresAt: Self.expiresAt
      )
    )
    XCTAssertEqual(store.saved, [record])
  }

  func testCommittedInstallationRecoversAfterFinalGrantSaveFailure() async throws {
    let store = MemoryGrantStore(grantSaveFailuresRemaining: 1)
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    var challengeCount = 0
    var installationCount = 0
    var delegationCount = 0
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: [
            "self": Self.relayPubkey,
            "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]],
          ]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        challengeCount += 1
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": challengeCount == 1 ? Self.firstChallengeId : Self.secondChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/installations"):
        installationCount += 1
        return Self.response(
          request,
          status: 201,
          json: [
            "installation_handle": Self.installationHandle,
            "endpoint_epoch": 1,
            "expires_at": Self.expiresAt,
          ]
        )
      case ("POST", "http://push.example/v1/delegations"):
        delegationCount += 1
        let body = try Self.body(request)
        XCTAssertEqual(body["generation"] as? Int, delegationCount)
        return Self.response(
          request,
          status: 201,
          json: ["endpoint_grant": "opaque-grant-\(delegationCount)"]
        )
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    do {
      _ = try await driver.enroll(
        deviceToken: Data((1...32).map(UInt8.init)),
        relayURL: Self.relayURL
      )
      XCTFail("Expected the injected local save failure")
    } catch {
      XCTAssertEqual((error as NSError).domain, "MemoryGrantStore")
    }
    XCTAssertEqual(store.pending.first?.delegationGeneration, 1)

    let recovered = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: Self.relayURL
    )

    XCTAssertEqual(installationCount, 1)
    XCTAssertEqual(delegationCount, 2)
    XCTAssertEqual(recovered.generation, 2)
    XCTAssertEqual(recovered.endpointGrant, "opaque-grant-2")
    XCTAssertTrue(store.pending.isEmpty)
  }

  func testCommittedInstallationRecoversAfterResponseLoss() async throws {
    let store = MemoryGrantStore()
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    var challengeCount = 0
    var installationCount = 0
    var firstInstallationBody: [String: Any]?
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: [
            "self": Self.relayPubkey,
            "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]],
          ]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        challengeCount += 1
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": challengeCount == 1 ? Self.firstChallengeId : Self.secondChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/installations"):
        installationCount += 1
        let body = try Self.body(request)
        if installationCount == 1 {
          firstInstallationBody = body
          throw URLError(.networkConnectionLost)
        }
        XCTAssertTrue(
          NSDictionary(dictionary: body).isEqual(to: try XCTUnwrap(firstInstallationBody))
        )
        return Self.response(
          request,
          status: 201,
          json: [
            "installation_handle": Self.installationHandle,
            "endpoint_epoch": 1,
            "expires_at": Self.expiresAt,
          ]
        )
      case ("POST", "http://push.example/v1/delegations"):
        return Self.response(
          request,
          status: 201,
          json: ["endpoint_grant": "opaque-grant"]
        )
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    do {
      _ = try await driver.enroll(
        deviceToken: Data((1...32).map(UInt8.init)),
        relayURL: Self.relayURL
      )
      XCTFail("Expected the simulated lost installation response")
    } catch {
      XCTAssertEqual((error as NSError).domain, NSURLErrorDomain)
    }
    XCTAssertNil(store.pending.first?.gatewayInstallationHandle)

    let recovered = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: Self.relayURL
    )

    XCTAssertEqual(challengeCount, 2)
    XCTAssertEqual(installationCount, 2)
    XCTAssertEqual(recovered.endpointGrant, "opaque-grant")
    XCTAssertTrue(store.pending.isEmpty)
  }

  func testRelayOriginPreservesNonDefaultPortWithoutTrailingSlash() async throws {
    let relayURL = URL(string: "wss://relay.example:8443/")!
    let store = MemoryGrantStore()
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    var challengeCount = 0
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example:8443/"):
        return Self.response(
          request,
          status: 200,
          json: [
            "self": Self.relayPubkey,
            "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]],
          ]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        challengeCount += 1
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": challengeCount == 1 ? Self.firstChallengeId : Self.secondChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/installations"):
        return Self.response(
          request,
          status: 201,
          json: [
            "installation_handle": Self.installationHandle,
            "endpoint_epoch": 1,
            "expires_at": Self.expiresAt,
          ]
        )
      case ("POST", "http://push.example/v1/delegations"):
        return Self.response(
          request,
          status: 201,
          json: ["endpoint_grant": "opaque-grant"]
        )
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    let record = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: relayURL
    )

    XCTAssertEqual(record.relayOrigin, "wss://relay.example:8443")
  }

  func testLegacyGrantWithoutGatewayOriginIsRejected() throws {
    let data = Data(
      #"{"relayOrigin":"wss://relay.example","relayPubkey":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","installationId":"000102030405060708090a0b0c0d0e0f","endpointGrant":"opaque","endpointHash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","appProfile":"buzz-ios-dogfood","endpointEpoch":1,"generation":1,"expiresAt":1752624000}"#
        .utf8
    )

    XCTAssertThrowsError(try JSONDecoder().decode(BuzzPushEndpointGrantRecord.self, from: data))
  }

  func testGrantWithoutAppAttestKeyIsRejected() throws {
    let data = Data(
      #"{"gatewayOrigin":"https://push.example","relayOrigin":"wss://relay.example","relayPubkey":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","installationId":"000102030405060708090a0b0c0d0e0f","endpointGrant":"opaque","endpointHash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","appProfile":"buzz-ios-dogfood","endpointEpoch":1,"generation":1,"expiresAt":1752624000}"#
        .utf8
    )

    XCTAssertThrowsError(try JSONDecoder().decode(BuzzPushEndpointGrantRecord.self, from: data))
  }

  func testDriverMovesGrantAndPendingStateFromAnotherGatewayIntoCleanupJournal() throws {
    let record = BuzzPushEndpointGrantRecord(
      gatewayOrigin: "https://old-gateway.example",
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "old-grant",
      endpointHash: String(repeating: "b", count: 64),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let pending = BuzzPushPendingEnrollmentRecord(
      gatewayOrigin: "https://old-gateway.example",
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      endpointHash: String(repeating: "b", count: 64),
      appProfile: "buzz-ios-dogfood",
      expiresAt: Self.expiresAt,
      installationId: Self.installationId
    )
    let store = MemoryGrantStore(records: [record], pending: [pending])

    _ = try makeDriver(store: store, appAttest: RecordingAppAttest())

    XCTAssertEqual(store.saved, [])
    XCTAssertTrue(store.pending.isEmpty)
    XCTAssertEqual(
      store.resetOperations, ["cleanup:https://old-gateway.example", "records", "pending"])
    XCTAssertEqual(
      store.cleanup,
      [
        BuzzPushGatewayCleanupState(
          gatewayOrigin: "https://old-gateway.example",
          grants: [record],
          pendingEnrollments: [pending]
        )
      ]
    )
  }

  func testGatewayResetRestoresCurrentCleanupBeforeRemovingJournal() throws {
    let record = BuzzPushEndpointGrantRecord(
      gatewayOrigin: "https://gateway-a.example",
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "gateway-a-grant",
      endpointHash: String(repeating: "b", count: 64),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [record])

    try store.reset(forGatewayOrigin: "https://gateway-b.example")
    store.resetOperations.removeAll()
    try store.reset(forGatewayOrigin: "https://gateway-a.example")

    XCTAssertEqual(store.saved, [record])
    XCTAssertTrue(store.cleanup.isEmpty)
    XCTAssertEqual(store.resetOperations, ["records", "cleanup-removed:https://gateway-a.example"])
  }

  func testRestoredResponseLostEnrollmentIsRevokedBeforeReplacement() async throws {
    let oldToken = Data(repeating: 0x07, count: 32)
    let newToken = Data(repeating: 0x08, count: 32)
    let quarantinedHandle = "11111111-1111-4111-8111-111111111111"
    let quarantined = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://other-relay.example",
      relayPubkey: String(repeating: "b", count: 64),
      gatewayInstallationHandle: quarantinedHandle,
      appAttestKeyId: Self.keyId,
      installationId: "101112131415161718191a1b1c1d1e1f",
      endpointGrant: "quarantined-grant",
      endpointHash: Self.hex(SHA256.hash(data: oldToken)),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let pending = BuzzPushPendingEnrollmentRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      endpoint: Self.hex(oldToken),
      endpointHash: Self.hex(SHA256.hash(data: oldToken)),
      appProfile: "buzz-ios-dogfood",
      expiresAt: Self.expiresAt,
      installationId: Self.installationId,
      challengeId: Self.firstChallengeId,
      challenge: Self.challenge,
      keyId: Self.keyId,
      attestation: Self.attestation
    )
    let store = MemoryGrantStore()
    store.cleanup = [
      BuzzPushGatewayCleanupState(
        gatewayOrigin: Self.gatewayOrigin,
        grants: [quarantined],
        pendingEnrollments: [pending],
        revocationPendingInstallationHandles: [quarantinedHandle]
      )
    ]
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    var challengeRequests = 0
    var replayedOldEndpoint = false
    var revokedHandles = Set<String>()
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: [
            "self": Self.relayPubkey,
            "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]],
          ]
        )
      case ("POST", "http://push.example/v1/installations"):
        replayedOldEndpoint = try Self.body(request)["endpoint"] as? String == Self.hex(oldToken)
        return Self.response(
          request,
          status: 201,
          json: [
            "installation_handle": Self.installationHandle,
            "endpoint_epoch": 1,
            "expires_at": Self.expiresAt,
          ]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        challengeRequests += 1
        guard challengeRequests <= 2 else {
          return Self.response(request, status: 503, json: ["error": "injected"])
        }
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.secondChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/installations/revoke"):
        revokedHandles.insert(try XCTUnwrap(Self.body(request)["installation_handle"] as? String))
        return Self.response(request, status: 200, json: ["status": "revoked"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    do {
      _ = try await driver.enroll(deviceToken: newToken, relayURL: Self.relayURL)
      XCTFail("Expected the injected replacement challenge failure")
    } catch BuzzDevPushEnrollmentError.unexpectedStatus(
      route: "v1/installations/challenges", expected: 200, actual: 503, _
    ) {
      // The old installation was reconciled before replacement began.
    }

    XCTAssertTrue(replayedOldEndpoint)
    XCTAssertEqual(revokedHandles, [quarantinedHandle, Self.installationHandle])
    XCTAssertEqual(store.saved, [])
    XCTAssertTrue(store.pending.isEmpty)
    XCTAssertTrue(store.cleanup.isEmpty)
  }

  func testEndpointChangeJournalsEveryGrantSharingRevokedInstallation() async throws {
    let oldToken = Data(repeating: 0x07, count: 32)
    let newToken = Data(repeating: 0x08, count: 32)
    let endpointHash = Self.hex(SHA256.hash(data: oldToken))
    func grant(relayOrigin: String, relayPubkey: String) -> BuzzPushEndpointGrantRecord {
      BuzzPushEndpointGrantRecord(
        gatewayOrigin: Self.gatewayOrigin,
        relayOrigin: relayOrigin,
        relayPubkey: relayPubkey,
        gatewayInstallationHandle: Self.installationHandle,
        appAttestKeyId: Self.keyId,
        installationId: Self.installationId,
        endpointGrant: "grant-\(relayOrigin)",
        endpointHash: endpointHash,
        appProfile: "buzz-ios-dogfood",
        endpointEpoch: 1,
        generation: 1,
        expiresAt: Self.expiresAt
      )
    }
    let primary = grant(relayOrigin: "wss://relay.example", relayPubkey: Self.relayPubkey)
    let shared = grant(
      relayOrigin: "wss://shared-relay.example",
      relayPubkey: String(repeating: "c", count: 64)
    )
    let pending = BuzzPushPendingEnrollmentRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: primary.relayOrigin,
      relayPubkey: primary.relayPubkey,
      endpoint: Self.hex(oldToken),
      endpointHash: endpointHash,
      appProfile: primary.appProfile,
      expiresAt: Self.expiresAt,
      installationId: primary.installationId,
      gatewayInstallationHandle: Self.installationHandle,
      keyId: Self.keyId,
      delegationGeneration: 2
    )
    let store = MemoryGrantStore(records: [primary, shared], pending: [pending])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    var challengeRequests = 0
    var revoked = false
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: ["push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]]]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        challengeRequests += 1
        guard challengeRequests == 1 else {
          return Self.response(request, status: 503, json: ["error": "injected"])
        }
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/installations/revoke"):
        let body = try Self.body(request)
        XCTAssertEqual(body["installation_handle"] as? String, Self.installationHandle)
        revoked = true
        return Self.response(request, status: 200, json: ["status": "revoked"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    do {
      _ = try await driver.enroll(deviceToken: newToken, relayURL: Self.relayURL)
      XCTFail("Expected the injected replacement challenge failure")
    } catch BuzzDevPushEnrollmentError.unexpectedStatus(
      route: "v1/installations/challenges", expected: 200, actual: 503, _
    ) {
      // Every grant sharing the revoked installation was removed first.
    }

    XCTAssertTrue(revoked)
    XCTAssertTrue(store.saved.isEmpty)
    XCTAssertTrue(store.pending.isEmpty)
    XCTAssertTrue(store.cleanup.isEmpty)
    XCTAssertEqual(
      store.replacementOrigins,
      ["wss://relay.example", "wss://shared-relay.example"]
    )
  }

  func testRelayRotationRevokesKnownCommittedGenerationWhenReservedGenerationDidNotCommit()
    async throws
  {
    let newRelayPubkey = String(repeating: "b", count: 64)
    let endpointHash = Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init))))
    let existing = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      relayMetadataPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "existing-grant",
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let pending = BuzzPushPendingEnrollmentRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      endpoint: Self.endpoint,
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      expiresAt: Self.expiresAt,
      installationId: Self.installationId,
      gatewayInstallationHandle: Self.installationHandle,
      keyId: Self.keyId,
      delegationGeneration: 2
    )
    let unrelatedHigherGeneration = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://other-relay.example",
      relayPubkey: String(repeating: "c", count: 64),
      relayMetadataPubkey: String(repeating: "c", count: 64),
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: "101112131415161718191a1b1c1d1e1f",
      endpointGrant: "other-relay-grant",
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 7,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(
      records: [unrelatedHigherGeneration, existing],
      pending: [pending],
      pendingRemoveFailuresRemaining: 1
    )
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    var challengeRequests = 0
    var revokedGenerations: [Int] = []
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: [
            "self": newRelayPubkey,
            "push": ["keys": [["pubkey": newRelayPubkey, "current": true]]],
          ]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        challengeRequests += 1
        guard challengeRequests <= 2 else {
          return Self.response(request, status: 503, json: ["error": "injected"])
        }
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id":
              challengeRequests == 1 ? Self.firstChallengeId : Self.secondChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/delegations/revoke"):
        let body = try Self.body(request)
        XCTAssertEqual(body["installation_handle"] as? String, Self.installationHandle)
        XCTAssertEqual(body["relay_pubkey"] as? String, Self.relayPubkey)
        let generation = try XCTUnwrap(body["generation"] as? Int)
        revokedGenerations.append(generation)
        if generation == 2 {
          return Self.response(request, status: 404, json: ["error": "not_authorized"])
        }
        XCTAssertEqual(generation, 1)
        return Self.response(request, status: 200, json: ["status": "revoked"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    do {
      _ = try await driver.enroll(
        deviceToken: Data((1...32).map(UInt8.init)),
        relayURL: Self.relayURL
      )
      XCTFail("Expected the injected pending-journal deletion failure")
    } catch let error as NSError where error.domain == "MemoryGrantStore" && error.code == 3 {
      // The completed revocation remains journaled for a local-only retry.
    }

    XCTAssertEqual(store.saved, [unrelatedHigherGeneration])
    XCTAssertEqual(store.pending.first?.delegationRevoked, true)

    do {
      _ = try await driver.enroll(
        deviceToken: Data((1...32).map(UInt8.init)),
        relayURL: Self.relayURL
      )
      XCTFail("Expected the injected replacement delegation challenge failure")
    } catch BuzzDevPushEnrollmentError.unexpectedStatus(
      route: "v1/installations/challenges", expected: 200, actual: 503, _
    ) {
      // The known committed generation was revoked before replacement began.
    }

    XCTAssertEqual(revokedGenerations, [2, 1])
    XCTAssertEqual(store.saved, [unrelatedHigherGeneration])
    XCTAssertEqual(store.pending.count, 1)
    XCTAssertEqual(store.pending.first?.relayPubkey, newRelayPubkey)
    XCTAssertEqual(store.pending.first?.gatewayInstallationHandle, Self.installationHandle)
  }

  func testRelayRotationPreservesDelegationUsedBySiblingOrigin() async throws {
    let newRelayPubkey = String(repeating: "b", count: 64)
    let endpointHash = Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init))))
    let existing = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "existing-grant",
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let sibling = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://sibling.example",
      relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: "303132333435363738393a3b3c3d3e3f",
      endpointGrant: "sibling-grant",
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let pending = BuzzPushPendingEnrollmentRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      endpoint: Self.endpoint,
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      expiresAt: Self.expiresAt,
      installationId: Self.installationId,
      gatewayInstallationHandle: Self.installationHandle,
      keyId: Self.keyId,
      delegationGeneration: 2
    )
    let store = MemoryGrantStore(records: [sibling, existing], pending: [pending])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: [
            "self": newRelayPubkey,
            "push": ["keys": [["pubkey": newRelayPubkey, "current": true]]],
          ]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/delegations"):
        let body = try Self.body(request)
        XCTAssertEqual(body["relay_pubkey"] as? String, newRelayPubkey)
        return Self.response(
          request,
          status: 201,
          json: ["endpoint_grant": "replacement-grant"]
        )
      case ("POST", "http://push.example/v1/delegations/revoke"):
        XCTFail("A delegation still used by a sibling origin must not be revoked")
        return Self.response(request, status: 500, json: [:])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    let replacement = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: Self.relayURL
    )

    XCTAssertEqual(replacement.relayPubkey, newRelayPubkey)
    XCTAssertTrue(store.saved.contains(sibling))
    XCTAssertEqual(store.replacementOrigins, ["wss://sibling.example"])
    XCTAssertEqual(
      store.saved.filter { $0.relayOrigin == "wss://relay.example" }.map(\.relayPubkey),
      [newRelayPubkey]
    )
  }

  func testCleanupRevokesAndDeletesStaleGatewaysWithoutRelayEnrollment() async throws {
    let endpointHash = Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init))))
    let current = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "current-grant",
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let staleHandle = "44444444-4444-4444-8444-444444444444"
    let staleKeyId = Data(repeating: 0xBB, count: 32).base64EncodedString()
    let stale = BuzzPushEndpointGrantRecord(
      gatewayOrigin: "http://old-gateway.example",
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: staleHandle,
      appAttestKeyId: staleKeyId,
      installationId: Self.installationId,
      endpointGrant: "stale-grant",
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let secondStaleHandle = "55555555-5555-4555-8555-555555555555"
    let secondStaleKeyId = Data(repeating: 0xCC, count: 32).base64EncodedString()
    let secondStale = BuzzPushEndpointGrantRecord(
      gatewayOrigin: "http://old-gateway.example",
      relayOrigin: "wss://second-relay.example",
      relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: secondStaleHandle,
      appAttestKeyId: secondStaleKeyId,
      installationId: "101112131415161718191a1b1c1d1e1f",
      endpointGrant: "second-stale-grant",
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 4,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [current, stale, secondStale])
    let appAttest = RecordingAppAttest()
    let driver = try makeDriver(store: store, appAttest: appAttest)
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: ["push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]]]
        )
      case ("POST", "http://old-gateway.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://old-gateway.example/v1/installations/revoke"):
        let body = try Self.body(request)
        let handle = body["installation_handle"] as? String
        XCTAssertTrue([staleHandle, secondStaleHandle].contains(handle))
        if handle == staleHandle {
          XCTAssertEqual(body["endpoint_epoch"] as? Int, 1)
          XCTAssertEqual(body["new_endpoint_epoch"] as? Int, 2)
        } else {
          XCTAssertEqual(body["endpoint_epoch"] as? Int, 4)
          XCTAssertEqual(body["new_endpoint_epoch"] as? Int, 5)
        }
        return Self.response(request, status: 200, json: ["status": "revoked"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    try await driver.cleanRetiredGateways()

    XCTAssertEqual(store.saved, [current])
    XCTAssertTrue(store.cleanup.isEmpty)
    XCTAssertEqual(
      Set(appAttest.assertionKeyIds.compactMap { $0 }), [staleKeyId, secondStaleKeyId])
  }

  func testCleanupReplaysProtectedEndpointAfterCurrentTokenChanges() async throws {
    let oldToken = Data(repeating: 0x07, count: 32)
    let newToken = Data(repeating: 0x08, count: 32)
    let oldGatewayURL = URL(string: "http://old-gateway.example")!
    let store = MemoryGrantStore()
    let oldDriver = try makeDriver(
      gatewayBaseURL: oldGatewayURL,
      store: store,
      appAttest: RecordingAppAttest()
    )
    var challengeRequests = 0
    var installationRequests = 0
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: ["push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]]]
        )
      case ("POST", "http://old-gateway.example/v1/installations/challenges"):
        challengeRequests += 1
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": challengeRequests == 1 ? Self.firstChallengeId : Self.secondChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://old-gateway.example/v1/installations"):
        installationRequests += 1
        let body = try Self.body(request)
        XCTAssertEqual(body["endpoint"] as? String, Self.hex(oldToken))
        if installationRequests == 1 {
          throw URLError(.networkConnectionLost)
        }
        return Self.response(
          request,
          status: 201,
          json: [
            "installation_handle": Self.installationHandle,
            "endpoint_epoch": 1,
            "expires_at": Self.expiresAt,
          ]
        )
      case ("POST", "http://old-gateway.example/v1/installations/revoke"):
        return Self.response(request, status: 200, json: ["status": "revoked"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    do {
      _ = try await oldDriver.enroll(deviceToken: oldToken, relayURL: Self.relayURL)
      XCTFail("Expected the committed enrollment response to be lost")
    } catch {
      XCTAssertEqual(store.pending.first?.endpoint, Self.hex(oldToken))
    }

    let currentDriver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    try await currentDriver.cleanRetiredGateways(deviceToken: newToken)

    XCTAssertEqual(installationRequests, 2)
    XCTAssertTrue(store.cleanup.isEmpty)
  }

  func testLegacyCleanupWaitsForAPNsTokenBeforeReplayingHashedEndpoint() async throws {
    let token = Data(repeating: 0x07, count: 32)
    let oldOrigin = "http://old-gateway.example"
    let pending = BuzzPushPendingEnrollmentRecord(
      gatewayOrigin: oldOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      endpointHash: Self.hex(SHA256.hash(data: token)),
      appProfile: "buzz-ios-dogfood",
      expiresAt: Self.expiresAt,
      installationId: Self.installationId,
      challengeId: Self.firstChallengeId,
      challenge: Self.challenge,
      keyId: Self.keyId,
      attestation: Self.attestation
    )
    let state = BuzzPushGatewayCleanupState(
      gatewayOrigin: oldOrigin,
      grants: [],
      pendingEnrollments: [pending]
    )
    let store = MemoryGrantStore()
    store.cleanup = [state]
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      XCTFail(
        "Cleanup must wait for APNs before requesting \(request.url?.absoluteString ?? "nil")")
      return Self.response(request, status: 500, json: [:])
    }

    do {
      try await driver.cleanRetiredGateways()
      XCTFail("Expected endpoint-less legacy cleanup to remain queued")
    } catch {
      XCTAssertEqual(
        error as? BuzzDevPushEnrollmentError,
        .retiredGatewayCleanupIncomplete
      )
    }
    XCTAssertEqual(store.cleanup, [state])

    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("POST", "http://old-gateway.example/v1/installations"):
        XCTAssertEqual(try Self.body(request)["endpoint"] as? String, Self.hex(token))
        return Self.response(
          request,
          status: 201,
          json: [
            "installation_handle": Self.installationHandle,
            "endpoint_epoch": 1,
            "expires_at": Self.expiresAt,
          ]
        )
      case ("POST", "http://old-gateway.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.secondChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://old-gateway.example/v1/installations/revoke"):
        return Self.response(request, status: 200, json: ["status": "revoked"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    try await driver.cleanRetiredGateways(deviceToken: token)
    XCTAssertTrue(store.cleanup.isEmpty)
  }

  func testFailedStaleGatewayRevocationKeepsCleanupJournal() async throws {
    let endpointHash = Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init))))
    let stale = BuzzPushEndpointGrantRecord(
      gatewayOrigin: "http://old-gateway.example",
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: "44444444-4444-4444-8444-444444444444",
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "stale-grant",
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [stale])
    let appAttest = RecordingAppAttest()
    let driver = try makeDriver(store: store, appAttest: appAttest)
    URLProtocolStub.handler = { request in
      XCTAssertNotEqual(request.url?.absoluteString, "https://relay.example/")
      return Self.response(request, status: 503, json: ["error": "unavailable"])
    }

    do {
      try await driver.cleanRetiredGateways(deviceToken: Data((1...32).map(UInt8.init)))
      XCTFail("Expected retired gateway cleanup to remain queued")
    } catch {
      XCTAssertEqual(
        error as? BuzzDevPushEnrollmentError,
        .retiredGatewayCleanupIncomplete
      )
    }

    XCTAssertTrue(store.saved.isEmpty)
    XCTAssertEqual(try XCTUnwrap(store.cleanup.first).grants, [stale])
    XCTAssertTrue(appAttest.preparedAttestations.isEmpty)
  }

  func testCleanupCheckpointsEachRevokedInstallationBeforeLaterFailure() async throws {
    let endpointHash = Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init))))
    func staleRecord(handle: String, relayOrigin: String) -> BuzzPushEndpointGrantRecord {
      BuzzPushEndpointGrantRecord(
        gatewayOrigin: "http://old-gateway.example",
        relayOrigin: relayOrigin,
        relayPubkey: Self.relayPubkey,
        gatewayInstallationHandle: handle,
        appAttestKeyId: Self.keyId,
        installationId: Self.installationId,
        endpointGrant: "stale-grant-\(handle)",
        endpointHash: endpointHash,
        appProfile: "buzz-ios-dogfood",
        endpointEpoch: 1,
        generation: 1,
        expiresAt: Self.expiresAt
      )
    }
    let revokedHandle = "44444444-4444-4444-8444-444444444444"
    let failedHandle = "55555555-5555-4555-8555-555555555555"
    let revoked = staleRecord(handle: revokedHandle, relayOrigin: "wss://first.example")
    let failed = staleRecord(handle: failedHandle, relayOrigin: "wss://second.example")
    let store = MemoryGrantStore(records: [revoked, failed])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("POST", "http://old-gateway.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://old-gateway.example/v1/installations/revoke"):
        let body = try Self.body(request)
        if body["installation_handle"] as? String == revokedHandle {
          return Self.response(request, status: 200, json: ["status": "revoked"])
        }
        XCTAssertEqual(body["installation_handle"] as? String, failedHandle)
        return Self.response(request, status: 503, json: ["error": "unavailable"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    do {
      try await driver.cleanRetiredGateways()
      XCTFail("Expected the later revocation failure to keep cleanup queued")
    } catch {
      XCTAssertEqual(
        error as? BuzzDevPushEnrollmentError,
        .retiredGatewayCleanupIncomplete
      )
    }

    XCTAssertTrue(store.saved.isEmpty)
    XCTAssertEqual(try XCTUnwrap(store.cleanup.first).grants, [failed])
  }

  func testCleanupKeepsJournalWhenRevocationReturnsAmbiguous404() async throws {
    let endpointHash = Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init))))
    let stale = BuzzPushEndpointGrantRecord(
      gatewayOrigin: "http://old-gateway.example",
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: "44444444-4444-4444-8444-444444444444",
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "stale-grant",
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [stale])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("POST", "http://old-gateway.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://old-gateway.example/v1/installations/revoke"):
        return Self.response(request, status: 404, json: ["error": "not_authorized"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    do {
      try await driver.cleanRetiredGateways()
      XCTFail("Expected ambiguous revocation failure to keep cleanup queued")
    } catch {
      XCTAssertEqual(
        error as? BuzzDevPushEnrollmentError,
        .retiredGatewayCleanupIncomplete
      )
    }

    XCTAssertTrue(store.saved.isEmpty)
    XCTAssertEqual(try XCTUnwrap(store.cleanup.first).grants, [stale])
  }

  func testRollbackCannotRestoreGrantAfterRevocationCheckpointFailure() async throws {
    let stale = BuzzPushEndpointGrantRecord(
      gatewayOrigin: "http://old-gateway.example",
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: "44444444-4444-4444-8444-444444444444",
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "stale-grant",
      endpointHash: String(repeating: "b", count: 64),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    // Reset journals the retired gateway on save 1, revocation intent is save
    // 2, and save 3 is the injected post-revocation checkpoint failure.
    let store = MemoryGrantStore(records: [stale], cleanupSaveFailureCalls: [3])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("POST", "http://old-gateway.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://old-gateway.example/v1/installations/revoke"):
        return Self.response(request, status: 200, json: ["status": "revoked"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    do {
      try await driver.cleanRetiredGateways()
      XCTFail("Expected the post-revocation checkpoint to fail")
    } catch {
      XCTAssertEqual(
        error as? BuzzDevPushEnrollmentError,
        .retiredGatewayCleanupIncomplete
      )
    }

    let cleanup = try XCTUnwrap(store.cleanup.first)
    XCTAssertEqual(cleanup.grants, [stale])
    XCTAssertEqual(
      cleanup.revocationPendingInstallationHandles,
      [try XCTUnwrap(stale.gatewayInstallationHandle)]
    )

    store.cleanupSaveFailureCalls = []
    try store.reset(forGatewayOrigin: stale.gatewayOrigin)

    XCTAssertTrue(store.saved.isEmpty)
    XCTAssertEqual(store.cleanup.first?.grants, [stale])
  }

  func testCleanupContinuesAfterAnEarlierRetiredGatewayFails() async throws {
    let endpointHash = Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init))))
    func staleRecord(origin: String, handle: String) -> BuzzPushEndpointGrantRecord {
      BuzzPushEndpointGrantRecord(
        gatewayOrigin: origin,
        relayOrigin: "wss://relay.example",
        relayPubkey: Self.relayPubkey,
        gatewayInstallationHandle: handle,
        appAttestKeyId: Self.keyId,
        installationId: Self.installationId,
        endpointGrant: "stale-grant",
        endpointHash: endpointHash,
        appProfile: "buzz-ios-dogfood",
        endpointEpoch: 1,
        generation: 1,
        expiresAt: Self.expiresAt
      )
    }
    let offlineOrigin = "http://offline-gateway.example"
    let reachableOrigin = "http://reachable-gateway.example"
    let offline = BuzzPushGatewayCleanupState(
      gatewayOrigin: offlineOrigin,
      grants: [
        staleRecord(
          origin: offlineOrigin,
          handle: "44444444-4444-4444-8444-444444444444"
        )
      ],
      pendingEnrollments: []
    )
    let reachable = BuzzPushGatewayCleanupState(
      gatewayOrigin: reachableOrigin,
      grants: [
        staleRecord(
          origin: reachableOrigin,
          handle: "55555555-5555-4555-8555-555555555555"
        )
      ],
      pendingEnrollments: []
    )
    let store = MemoryGrantStore()
    store.cleanup = [offline, reachable]
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    var reachableRevoked = false
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("POST", "http://offline-gateway.example/v1/installations/challenges"):
        return Self.response(request, status: 503, json: ["error": "unavailable"])
      case ("POST", "http://reachable-gateway.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://reachable-gateway.example/v1/installations/revoke"):
        reachableRevoked = true
        return Self.response(request, status: 200, json: ["status": "revoked"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    do {
      try await driver.cleanRetiredGateways()
      XCTFail("Expected the offline gateway cleanup to remain queued")
    } catch {
      XCTAssertEqual(
        error as? BuzzDevPushEnrollmentError,
        .retiredGatewayCleanupIncomplete
      )
    }

    XCTAssertTrue(reachableRevoked)
    XCTAssertEqual(store.cleanup.map(\.gatewayOrigin), [offlineOrigin])
    XCTAssertEqual(store.cleanup.first?.grants, offline.grants)
    XCTAssertEqual(
      store.cleanup.first?.revocationPendingInstallationHandles,
      ["44444444-4444-4444-8444-444444444444"]
    )
  }

  func testRealAppAttestFailsLoudlyWhenUnsupported() async throws {
    let service = RecordingDCAppAttestService(isSupported: false)
    let provider = BuzzDCAppAttestProvider(
      service: service,
      keyIdStore: MemoryAppAttestKeyIdStore(keyId: Self.keyId)
    )

    do {
      _ = try await provider.prepareAttestation()
      XCTFail("Expected App Attest to be unavailable")
    } catch {
      XCTAssertEqual(error as? BuzzDevPushEnrollmentError, .appAttestUnsupported)
    }
    XCTAssertEqual(service.generateKeyCallCount, 0)
  }

  func testRealAppAttestGeneratesPersistsAndMapsAttestation() async throws {
    let service = RecordingDCAppAttestService(
      generatedKeyId: Self.keyId,
      attestationObject: Data([0x01, 0x02, 0x03])
    )
    let keyIdStore = MemoryAppAttestKeyIdStore()
    let provider = BuzzDCAppAttestProvider(service: service, keyIdStore: keyIdStore)
    let clientData = Data("enrollment transcript".utf8)

    let prepared = try await provider.prepareAttestation()
    let attestation = try await provider.attestation(prepared, clientData: clientData)

    XCTAssertEqual(prepared, BuzzDevAttestation(keyId: Self.keyId, attestation: ""))
    XCTAssertEqual(keyIdStore.savedKeyIds, [Self.keyId])
    XCTAssertEqual(attestation.keyId, Self.keyId)
    XCTAssertEqual(attestation.attestation, Data([0x01, 0x02, 0x03]).base64EncodedString())
    XCTAssertEqual(service.attestedKeyIds, [Self.keyId])
    XCTAssertEqual(
      service.attestationClientDataHashes,
      [Data(SHA256.hash(data: clientData))]
    )
  }

  func testRealAppAttestAssertionUsesRequestedKeyAndMapsObject() async throws {
    let service = RecordingDCAppAttestService(assertionObject: Data([0x04, 0x05, 0x06]))
    let keyIdStore = MemoryAppAttestKeyIdStore(keyId: Self.keyId)
    let provider = BuzzDCAppAttestProvider(service: service, keyIdStore: keyIdStore)
    let clientData = Data("delegation transcript".utf8)

    let assertion = try await provider.assertion(keyId: Self.keyId, clientData: clientData)

    XCTAssertEqual(assertion, Data([0x04, 0x05, 0x06]).base64EncodedString())
    XCTAssertEqual(service.assertedKeyIds, [Self.keyId])
    XCTAssertEqual(
      service.assertionClientDataHashes,
      [Data(SHA256.hash(data: clientData))]
    )
    XCTAssertEqual(service.generateKeyCallCount, 0)
  }

  func testRealAppAttestAssertionUsesInstallationKeyInsteadOfLatestStoredKey() async throws {
    let installationKeyId = Data(repeating: 0xBB, count: 32).base64EncodedString()
    let service = RecordingDCAppAttestService(assertionObject: Data([0x07]))
    let provider = BuzzDCAppAttestProvider(
      service: service,
      keyIdStore: MemoryAppAttestKeyIdStore(keyId: Self.keyId)
    )

    _ = try await provider.assertion(
      keyId: installationKeyId,
      clientData: Data("retired installation transcript".utf8)
    )

    XCTAssertEqual(service.assertedKeyIds, [installationKeyId])
  }

  func testRealAppAttestRejectsInvalidGeneratedKeyBeforePersistence() async throws {
    for invalidKeyId in [
      "not-a-key-id",
      String(Self.keyId.dropLast(2)) + "p=",
      Data(repeating: 0xAA, count: 31).base64EncodedString(),
      Data(repeating: 0xAA, count: 33).base64EncodedString(),
    ] {
      let service = RecordingDCAppAttestService(generatedKeyId: invalidKeyId)
      let keyIdStore = MemoryAppAttestKeyIdStore()
      let provider = BuzzDCAppAttestProvider(service: service, keyIdStore: keyIdStore)

      do {
        _ = try await provider.prepareAttestation()
        XCTFail("Accepted invalid generated key ID: \(invalidKeyId)")
      } catch {
        XCTAssertEqual(error as? BuzzDevPushEnrollmentError, .invalidAppAttestKeyId)
      }
      XCTAssertTrue(keyIdStore.savedKeyIds.isEmpty)
    }
  }

  func testRealAppAttestRejectsMismatchedPreparedKey() async throws {
    let service = RecordingDCAppAttestService()
    let keyIdStore = MemoryAppAttestKeyIdStore(keyId: Self.keyId)
    let provider = BuzzDCAppAttestProvider(service: service, keyIdStore: keyIdStore)
    let otherKeyId = Data(repeating: 0xBB, count: 32).base64EncodedString()

    do {
      _ = try await provider.attestation(
        BuzzDevAttestation(keyId: otherKeyId, attestation: ""),
        clientData: Data("enrollment transcript".utf8)
      )
      XCTFail("Expected the prepared key ID to match persistent state")
    } catch {
      XCTAssertEqual(error as? BuzzDevPushEnrollmentError, .invalidAppAttestKeyId)
    }
    XCTAssertTrue(service.attestedKeyIds.isEmpty)
  }

  func testRealAppAttestForwardsServiceErrors() async throws {
    let expected = NSError(domain: "DeviceCheckTest", code: 41)
    let service = RecordingDCAppAttestService(error: expected)
    let provider = BuzzDCAppAttestProvider(
      service: service,
      keyIdStore: MemoryAppAttestKeyIdStore(keyId: Self.keyId)
    )

    do {
      _ = try await provider.assertion(
        keyId: Self.keyId,
        clientData: Data("delegation transcript".utf8)
      )
      XCTFail("Expected the DeviceCheck error")
    } catch {
      XCTAssertEqual((error as NSError).domain, expected.domain)
      XCTAssertEqual((error as NSError).code, expected.code)
    }
  }

  func testKeychainStoreReadsKeyIdAndIncludesAccessGroup() throws {
    var capturedQuery: [String: Any] = [:]
    let store = BuzzAppAttestKeyIdKeychainStore(
      accessGroup: "group.buzz",
      copyMatching: { query, result in
        capturedQuery = query as! [String: Any]
        result?.pointee = Data(Self.keyId.utf8) as CFData
        return errSecSuccess
      }
    )

    XCTAssertEqual(try store.keyId(), Self.keyId)
    XCTAssertEqual(
      capturedQuery[kSecClass as String] as? String, kSecClassGenericPassword as String)
    XCTAssertEqual(capturedQuery[kSecAttrService as String] as? String, "buzz.push.app-attest")
    XCTAssertEqual(capturedQuery[kSecAttrAccount as String] as? String, "key-id-v1")
    XCTAssertEqual(capturedQuery[kSecAttrAccessGroup as String] as? String, "group.buzz")
    XCTAssertEqual(capturedQuery[kSecReturnData as String] as? Bool, true)
    XCTAssertEqual(capturedQuery[kSecMatchLimit as String] as? String, kSecMatchLimitOne as String)
  }

  func testKeychainStoreReturnsNilOnMissAndRejectsInvalidData() throws {
    let missing = BuzzAppAttestKeyIdKeychainStore(
      accessGroup: nil,
      copyMatching: { _, _ in errSecItemNotFound }
    )
    XCTAssertNil(try missing.keyId())

    for invalidKeyId in [
      "bad",
      String(Self.keyId.dropLast(2)) + "p=",
      Data(repeating: 0xAA, count: 31).base64EncodedString(),
      Data(repeating: 0xAA, count: 33).base64EncodedString(),
    ] {
      let invalid = BuzzAppAttestKeyIdKeychainStore(
        accessGroup: nil,
        copyMatching: { _, result in
          result?.pointee = Data(invalidKeyId.utf8) as CFData
          return errSecSuccess
        }
      )
      XCTAssertThrowsError(try invalid.keyId(), "Accepted invalid key ID: \(invalidKeyId)") {
        XCTAssertEqual($0 as? BuzzDevPushEnrollmentError, .invalidAppAttestKeyId)
      }
    }
  }

  func testKeychainStoreUpdatesExistingKeyId() throws {
    var updatedQuery: [String: Any] = [:]
    var updatedValues: [String: Any] = [:]
    var addCallCount = 0
    let store = BuzzAppAttestKeyIdKeychainStore(
      accessGroup: nil,
      update: { query, values in
        updatedQuery = query as! [String: Any]
        updatedValues = values as! [String: Any]
        return errSecSuccess
      },
      add: { _, _ in
        addCallCount += 1
        return errSecSuccess
      }
    )

    try store.saveKeyId(Self.keyId)

    XCTAssertEqual(updatedQuery[kSecAttrService as String] as? String, "buzz.push.app-attest")
    XCTAssertEqual(updatedValues[kSecValueData as String] as? Data, Data(Self.keyId.utf8))
    XCTAssertEqual(addCallCount, 0)
  }

  func testKeychainStoreAddsMissingKeyIdWithDeviceOnlyAccessibility() throws {
    var addedItem: [String: Any] = [:]
    let store = BuzzAppAttestKeyIdKeychainStore(
      accessGroup: "group.buzz",
      update: { _, _ in errSecItemNotFound },
      add: { item, _ in
        addedItem = item as! [String: Any]
        return errSecSuccess
      }
    )

    try store.saveKeyId(Self.keyId)

    XCTAssertEqual(addedItem[kSecValueData as String] as? Data, Data(Self.keyId.utf8))
    XCTAssertEqual(
      addedItem[kSecAttrAccessible as String] as? String,
      kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String
    )
    XCTAssertEqual(addedItem[kSecAttrAccessGroup as String] as? String, "group.buzz")
  }

  func testKeychainStoreSurfacesReadUpdateAndAddErrors() throws {
    let readFailure = BuzzAppAttestKeyIdKeychainStore(
      accessGroup: nil,
      copyMatching: { _, _ in errSecInteractionNotAllowed }
    )
    XCTAssertThrowsError(try readFailure.keyId()) {
      XCTAssertEqual(($0 as NSError).code, Int(errSecInteractionNotAllowed))
    }

    let updateFailure = BuzzAppAttestKeyIdKeychainStore(
      accessGroup: nil,
      update: { _, _ in errSecInteractionNotAllowed }
    )
    XCTAssertThrowsError(try updateFailure.saveKeyId(Self.keyId)) {
      XCTAssertEqual(($0 as NSError).code, Int(errSecInteractionNotAllowed))
    }

    let addFailure = BuzzAppAttestKeyIdKeychainStore(
      accessGroup: nil,
      update: { _, _ in errSecItemNotFound },
      add: { _, _ in errSecDuplicateItem }
    )
    XCTAssertThrowsError(try addFailure.saveKeyId(Self.keyId)) {
      XCTAssertEqual(($0 as NSError).code, Int(errSecDuplicateItem))
    }
  }

  func testEnrollmentContinuesWhileRetiredGatewayCleanupRemainsQueued() async throws {
    let existing = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      relayMetadataPubkey: Self.relayPubkey,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "existing-grant",
      endpointHash: Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init)))),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [existing])
    store.cleanup = [
      BuzzPushGatewayCleanupState(
        gatewayOrigin: "http://retired-gateway.example",
        grants: [],
        pendingEnrollments: []
      )
    ]
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      guard request.httpMethod == "GET" else {
        XCTFail("Persisted grant reuse must not call the gateway")
        return Self.response(request, status: 500, json: [:])
      }
      return Self.response(
        request,
        status: 200,
        json: [
          "self": Self.relayPubkey,
          "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]],
        ]
      )
    }

    let record = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: Self.relayURL
    )

    XCTAssertEqual(record, existing)
    XCTAssertEqual(store.saved, [existing])
    XCTAssertEqual(store.cleanup.map(\.gatewayOrigin), ["http://retired-gateway.example"])
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testEnrollmentRetiresConflictingInstallationFromRenamedGateway() async throws {
    let oldGatewayOrigin = "http://old-gateway.example"
    let newGatewayURL = try XCTUnwrap(URL(string: "http://new-gateway.example"))
    let stale = BuzzPushEndpointGrantRecord(
      gatewayOrigin: oldGatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "stale-grant",
      endpointHash: Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init)))),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore()
    store.cleanup = [
      BuzzPushGatewayCleanupState(
        gatewayOrigin: oldGatewayOrigin,
        grants: [stale],
        pendingEnrollments: []
      )
    ]
    let driver = try makeDriver(
      gatewayBaseURL: newGatewayURL,
      store: store,
      appAttest: RecordingAppAttest()
    )
    var installationAttempts = 0
    var revokedOldInstallation = false
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: ["push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]]]
        )
      case ("POST", "http://new-gateway.example/v1/installations/challenges"),
        ("POST", "http://old-gateway.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://new-gateway.example/v1/installations"):
        installationAttempts += 1
        if installationAttempts == 1 {
          return Self.response(request, status: 409, json: ["error": "installation_conflict"])
        }
        return Self.response(
          request,
          status: 201,
          json: [
            "installation_handle": Self.installationHandle,
            "endpoint_epoch": 1,
            "expires_at": Self.expiresAt,
          ]
        )
      case ("POST", "http://old-gateway.example/v1/installations/revoke"):
        revokedOldInstallation = true
        return Self.response(request, status: 200, json: ["status": "revoked"])
      case ("POST", "http://new-gateway.example/v1/delegations"):
        return Self.response(request, status: 201, json: ["endpoint_grant": "new-grant"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    let record = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: Self.relayURL
    )

    XCTAssertEqual(installationAttempts, 2)
    XCTAssertTrue(revokedOldInstallation)
    XCTAssertTrue(store.cleanup.isEmpty)
    XCTAssertEqual(store.replacementOrigins, ["wss://relay.example"])
    XCTAssertEqual(record.gatewayOrigin, "http://new-gateway.example")
    XCTAssertEqual(record.endpointGrant, "new-grant")
  }

  func testEnrollmentRecoversQuarantinedLegacyInstallationWithOpaqueGrant() async throws {
    let store = MemoryGrantStore()
    store.legacyEndpointGrants = ["legacy-grant"]
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    var installationAttempts = 0
    var recoveryAttempts = 0
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: ["push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]]]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/installations"):
        installationAttempts += 1
        if installationAttempts == 1 {
          return Self.response(request, status: 409, json: ["error": "installation_conflict"])
        }
        return Self.response(
          request,
          status: 201,
          json: [
            "installation_handle": Self.installationHandle,
            "endpoint_epoch": 1,
            "expires_at": Self.expiresAt,
          ]
        )
      case ("POST", "http://push.example/v1/installations/recover"):
        recoveryAttempts += 1
        let body = try Self.body(request)
        XCTAssertEqual(body["endpoint_grant"] as? String, "legacy-grant")
        return Self.response(request, status: 200, json: ["status": "revoked"])
      case ("POST", "http://push.example/v1/delegations"):
        return Self.response(request, status: 201, json: ["endpoint_grant": "new-grant"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    let record = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: Self.relayURL
    )

    XCTAssertEqual(installationAttempts, 2)
    XCTAssertEqual(recoveryAttempts, 1)
    XCTAssertEqual(record.endpointGrant, "new-grant")
  }

  func testEnrollmentRecoversResponseLostLegacyPendingInstallation() async throws {
    let token = Data((1...32).map(UInt8.init))
    let store = MemoryGrantStore()
    store.legacyPendingEnrollments = [
      BuzzPushLegacyRecoveryInventory.BuzzPushLegacyPendingRecovery(
        relayOrigin: "wss://relay.example",
        endpointHash: Self.hex(SHA256.hash(data: token)),
        appProfile: "buzz-ios-dogfood",
        expiresAt: Self.expiresAt,
        challengeId: Self.firstChallengeId,
        challenge: Self.challenge,
        keyId: Self.keyId,
        attestation: Self.attestation
      )
    ]
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    var installationAttempts = 0
    var revokeAttempts = 0
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: ["push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]]]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/installations"):
        installationAttempts += 1
        if installationAttempts == 1 {
          return Self.response(request, status: 409, json: ["error": "installation_conflict"])
        }
        return Self.response(
          request,
          status: 201,
          json: [
            "installation_handle": Self.installationHandle,
            "endpoint_epoch": 1,
            "expires_at": Self.expiresAt,
          ]
        )
      case ("POST", "http://push.example/v1/installations/revoke"):
        revokeAttempts += 1
        return Self.response(request, status: 200, json: ["status": "revoked"])
      case ("POST", "http://push.example/v1/delegations"):
        return Self.response(request, status: 201, json: ["endpoint_grant": "new-grant"])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    let record = try await driver.enroll(deviceToken: token, relayURL: Self.relayURL)

    XCTAssertEqual(installationAttempts, 3)
    XCTAssertEqual(revokeAttempts, 1)
    XCTAssertEqual(record.endpointGrant, "new-grant")
  }

  func testSecondOriginOnSameRelayKeyReusesGrantWithFreshLeaseAddress() async throws {
    let existing = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://first.example",
      relayPubkey: Self.relayPubkey,
      relayMetadataPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: String(repeating: "f", count: 32),
      endpointGrant: "existing-grant",
      endpointHash: Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init)))),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 4,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [existing])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      XCTAssertEqual(request.httpMethod, "GET")
      XCTAssertEqual(request.url?.absoluteString, "https://second.example/")
      return Self.response(
        request,
        status: 200,
        json: [
          "self": Self.relayPubkey,
          "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]],
        ]
      )
    }

    let record = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: URL(string: "wss://second.example/")!
    )

    XCTAssertEqual(record.relayOrigin, "wss://second.example")
    XCTAssertEqual(record.gatewayInstallationHandle, Self.installationHandle)
    XCTAssertEqual(record.endpointGrant, existing.endpointGrant)
    XCTAssertEqual(record.generation, existing.generation)
    XCTAssertEqual(record.installationId, Self.installationId)
    XCTAssertNotEqual(record.installationId, existing.installationId)
    XCTAssertEqual(store.saved.count, 2)
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testSecondRelayKeyReusesAttestedInstallationAndCreatesOnlyDelegation() async throws {
    let secondRelayPubkey = String(repeating: "b", count: 64)
    let existing = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://first.example",
      relayPubkey: Self.relayPubkey,
      relayMetadataPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: String(repeating: "f", count: 32),
      endpointGrant: "first-relay-grant",
      endpointHash: Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init)))),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 7,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [existing])
    let appAttest = RecordingAppAttest()
    let driver = try makeDriver(store: store, appAttest: appAttest)
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://second.example/"):
        return Self.response(
          request,
          status: 200,
          json: [
            "self": secondRelayPubkey,
            "push": ["keys": [["pubkey": secondRelayPubkey, "current": true]]],
          ]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/delegations"):
        let body = try Self.body(request)
        XCTAssertEqual(body["installation_handle"] as? String, Self.installationHandle)
        XCTAssertEqual(body["relay_pubkey"] as? String, secondRelayPubkey)
        XCTAssertEqual(body["generation"] as? Int, 1)
        return Self.response(
          request,
          status: 201,
          json: ["endpoint_grant": "second-relay-grant"]
        )
      case ("POST", "http://push.example/v1/installations"):
        XCTFail("A second relay must not create a duplicate APNs installation")
        return Self.response(request, status: 500, json: [:])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    let record = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: URL(string: "wss://second.example/")!
    )

    XCTAssertEqual(record.gatewayInstallationHandle, Self.installationHandle)
    XCTAssertEqual(record.relayPubkey, secondRelayPubkey)
    XCTAssertEqual(record.endpointGrant, "second-relay-grant")
    XCTAssertEqual(record.generation, 1)
    XCTAssertEqual(appAttest.clientData.count, 1)
    XCTAssertEqual(store.saved.count, 2)
  }

  func testForcedRenewalBypassesReusableSiblingGrant() async throws {
    let existing = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      relayMetadataPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "invalidated-sibling-grant",
      endpointHash: Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init)))),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 4,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [existing])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: [
            "self": Self.relayPubkey,
            "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]],
          ]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/delegations"):
        let body = try Self.body(request)
        XCTAssertEqual(body["installation_handle"] as? String, Self.installationHandle)
        XCTAssertEqual(body["generation"] as? Int, 5)
        return Self.response(
          request,
          status: 201,
          json: ["endpoint_grant": "replacement-grant"]
        )
      case ("POST", "http://push.example/v1/installations"):
        XCTFail("Forced delegation renewal must reuse the existing installation")
        return Self.response(request, status: 500, json: [:])
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    let record = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: Self.relayURL,
      forceDelegationRenewal: true
    )

    XCTAssertEqual(record.gatewayInstallationHandle, Self.installationHandle)
    XCTAssertEqual(record.installationId, Self.installationId)
    XCTAssertEqual(record.generation, 5)
    XCTAssertEqual(record.endpointGrant, "replacement-grant")
  }

  func testForcedReplacementAdoptsNewerSiblingGrantWithoutRenewingAgain() async throws {
    let endpointHash = Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init))))
    let stale = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      relayMetadataPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "stale-grant",
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 4,
      expiresAt: Self.expiresAt
    )
    let renewed = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://sibling.example",
      relayPubkey: Self.relayPubkey,
      relayMetadataPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: "sibling-lease-address",
      endpointGrant: "renewed-shared-grant",
      endpointHash: endpointHash,
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 5,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [stale, renewed])
    let driver = try makeDriver(
      store: store,
      appAttest: RecordingAppAttest(),
      installationIdBytes: {
        XCTFail("A queued sibling must retain its existing lease address")
        return Data(repeating: 0xFF, count: 16)
      }
    )
    URLProtocolStub.handler = { request in
      guard request.httpMethod == "GET" else {
        XCTFail("A newer sibling grant must prevent another gateway renewal")
        return Self.response(request, status: 500, json: [:])
      }
      return Self.response(
        request,
        status: 200,
        json: [
          "self": Self.relayPubkey,
          "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]],
        ]
      )
    }

    let record = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: Self.relayURL,
      forceDelegationRenewal: true
    )

    XCTAssertEqual(record.installationId, Self.installationId)
    XCTAssertEqual(record.generation, 5)
    XCTAssertEqual(record.endpointGrant, "renewed-shared-grant")
  }

  func testExpiringGrantRenewsExistingInstallationAndReusesRelayLeaseAddress() async throws {
    let existing = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      relayMetadataPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "existing-grant",
      endpointHash: Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init)))),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 7,
      expiresAt: Self.now + 300
    )
    let store = MemoryGrantStore(records: [existing])
    let driver = try makeDriver(
      store: store,
      appAttest: RecordingAppAttest(),
      installationIdBytes: {
        XCTFail("Grant refresh must reuse the persisted installation id")
        return Data(repeating: 0xFF, count: 16)
      }
    )
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(
          request,
          status: 200,
          json: [
            "self": Self.relayPubkey,
            "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]],
          ]
        )
      case ("POST", "http://push.example/v1/installations/challenges"):
        return Self.response(
          request,
          status: 200,
          json: [
            "challenge_id": Self.firstChallengeId,
            "challenge": Self.challenge,
            "expires_at": Self.now + 300,
          ]
        )
      case ("POST", "http://push.example/v1/installations"):
        XCTFail("An expiring installation must renew through authenticated delegation")
        return Self.response(request, status: 500, json: [:])
      case ("POST", "http://push.example/v1/delegations"):
        let body = try Self.body(request)
        XCTAssertEqual(body["installation_handle"] as? String, Self.installationHandle)
        XCTAssertEqual(body["generation"] as? Int, 8)
        XCTAssertEqual(body["expires_at"] as? Int64, Self.expiresAt)
        return Self.response(
          request,
          status: 201,
          json: ["endpoint_grant": "refreshed-grant"]
        )
      default:
        XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
        return Self.response(request, status: 500, json: [:])
      }
    }

    let record = try await driver.enroll(
      deviceToken: Data((1...32).map(UInt8.init)),
      relayURL: Self.relayURL
    )

    XCTAssertEqual(record.installationId, Self.installationId)
    XCTAssertEqual(record.gatewayInstallationHandle, Self.installationHandle)
    XCTAssertEqual(record.generation, 8)
    XCTAssertEqual(record.expiresAt, Self.expiresAt)
    XCTAssertEqual(record.endpointGrant, "refreshed-grant")
  }

  func testRejectsMultipleCurrentRelayKeysBeforeGatewayEnrollment() async throws {
    let driver = try makeDriver(store: MemoryGrantStore(), appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      Self.response(
        request,
        status: 200,
        json: [
          "self": Self.relayPubkey,
          "push": [
            "keys": [
              ["pubkey": Self.relayPubkey, "current": true],
              ["pubkey": String(repeating: "b", count: 64), "current": true],
            ]
          ],
        ]
      )
    }

    do {
      _ = try await driver.enroll(deviceToken: Data([1]), relayURL: Self.relayURL)
      XCTFail("Expected an invalid relay descriptor")
    } catch {
      XCTAssertEqual(error as? BuzzDevPushEnrollmentError, .invalidRelayDescriptor)
    }
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testTracksRelayMetadataAuthoritySeparatelyFromPushDelegationKey() async throws {
    let pushPubkey = String(repeating: "b", count: 64)
    let oldMetadataPubkey = String(repeating: "c", count: 64)
    let deviceToken = Data((1...32).map(UInt8.init))
    let existing = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: pushPubkey,
      relayMetadataPubkey: oldMetadataPubkey,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "existing-grant",
      endpointHash: Self.hex(SHA256.hash(data: deviceToken)),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [existing])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      Self.response(
        request,
        status: 200,
        json: [
          "self": Self.relayPubkey,
          "push": [
            "keys": [
              ["pubkey": pushPubkey, "current": true]
            ]
          ],
        ]
      )
    }

    let record = try await driver.enroll(deviceToken: deviceToken, relayURL: Self.relayURL)

    XCTAssertEqual(record.relayPubkey, pushPubkey)
    XCTAssertEqual(record.relayMetadataPubkey, Self.relayPubkey)
    XCTAssertNotEqual(record.relayMetadataPubkey, oldMetadataPubkey)
    XCTAssertEqual(record.endpointGrant, existing.endpointGrant)
    XCTAssertEqual(store.saved, [record])
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testMissingRelayMetadataAuthorityDoesNotBlockExistingPushGrant() async throws {
    let deviceToken = Data((1...32).map(UInt8.init))
    let existing = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      relayMetadataPubkey: Self.relayPubkey,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "existing-grant",
      endpointHash: Self.hex(SHA256.hash(data: deviceToken)),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [existing])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      Self.response(
        request,
        status: 200,
        json: [
          "push": [
            "keys": [["pubkey": Self.relayPubkey, "current": true]]
          ]
        ]
      )
    }

    let record = try await driver.enroll(deviceToken: deviceToken, relayURL: Self.relayURL)

    XCTAssertEqual(record.relayPubkey, Self.relayPubkey)
    XCTAssertNil(record.relayMetadataPubkey)
    XCTAssertEqual(record.endpointGrant, existing.endpointGrant)
    XCTAssertEqual(store.saved, [record])
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testMalformedRelayMetadataAuthorityDoesNotBlockExistingPushGrant() async throws {
    let deviceToken = Data((1...32).map(UInt8.init))
    let existing = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin,
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      relayMetadataPubkey: Self.relayPubkey,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "existing-grant",
      endpointHash: Self.hex(SHA256.hash(data: deviceToken)),
      appProfile: "buzz-ios-dogfood",
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [existing])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      Self.response(
        request,
        status: 200,
        json: [
          "self": 42,
          "push": [
            "keys": [["pubkey": Self.relayPubkey, "current": true]]
          ],
        ]
      )
    }

    let record = try await driver.enroll(deviceToken: deviceToken, relayURL: Self.relayURL)

    XCTAssertEqual(record.relayPubkey, Self.relayPubkey)
    XCTAssertNil(record.relayMetadataPubkey)
    XCTAssertEqual(record.endpointGrant, existing.endpointGrant)
    XCTAssertEqual(store.saved, [record])
    XCTAssertEqual(URLProtocolStub.requests.count, 1)
  }

  func testFailsLoudlyOnUnexpectedGatewayStatus() async throws {
    let driver = try makeDriver(store: MemoryGrantStore(), appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      if request.httpMethod == "GET" {
        return Self.response(
          request,
          status: 200,
          json: [
            "self": Self.relayPubkey,
            "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]],
          ]
        )
      }
      return Self.response(request, status: 400, json: ["error": "invalid_request"])
    }

    do {
      _ = try await driver.enroll(deviceToken: Data([1]), relayURL: Self.relayURL)
      XCTFail("Expected the gateway error")
    } catch let error as BuzzDevPushEnrollmentError {
      XCTAssertEqual(
        error,
        .unexpectedStatus(
          route: "v1/installations/challenges",
          expected: 200,
          actual: 400,
          body: "{\"error\":\"invalid_request\"}"
        )
      )
    }
  }

  private func makeDriver(
    gatewayBaseURL: URL = BuzzDevPushEnrollmentDriverTests.gatewayURL,
    store: BuzzPushEndpointGrantStore,
    appAttest: BuzzDevAppAttesting,
    installationIdBytes: @escaping () throws -> Data = {
      Data(0..<16)
    }
  ) throws -> BuzzDevPushEnrollmentDriver {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [URLProtocolStub.self]
    return try BuzzDevPushEnrollmentDriver(
      gatewayBaseURL: gatewayBaseURL,
      store: store,
      session: URLSession(configuration: configuration),
      appAttest: appAttest,
      now: { Date(timeIntervalSince1970: TimeInterval(Self.now)) },
      lifetimeSeconds: Self.expiresAt - Self.now,
      installationIdBytes: installationIdBytes
    )
  }

  private func makeFixtureTranscript(
    name: String,
    replacements: [(String, String)]
  ) throws -> (bytes: Data, sha256: String) {
    let fixture = try Self.fixture()
    let vector = try XCTUnwrap(fixture.vectors.first { $0.name == name })
    let transcript = replacements.reduce(vector.transcript) {
      $0.replacingOccurrences(of: $1.0, with: $1.1)
    }
    return (Data(transcript.utf8), Self.hex(SHA256.hash(data: Data(transcript.utf8))))
  }

  private func assertMatchesVector(
    _ name: String,
    actual: Data,
    expectedSHA256: String,
    fixture: (bytes: Data, sha256: String),
    file: StaticString = #filePath,
    line: UInt = #line
  ) throws {
    XCTAssertEqual(
      fixture.sha256,
      expectedSHA256,
      "\(name) substituted gateway vector SHA-256",
      file: file,
      line: line
    )
    XCTAssertEqual(
      actual, fixture.bytes, "\(name) exact transcript bytes", file: file, line: line)
    XCTAssertEqual(
      Self.hex(SHA256.hash(data: actual)),
      fixture.sha256,
      "\(name) transcript SHA-256",
      file: file,
      line: line
    )
  }

  private struct Fixture: Decodable {
    struct Vector: Decodable {
      let name: String
      let transcript: String
    }
    let vectors: [Vector]
  }

  private static func fixture() throws -> Fixture {
    let path = try XCTUnwrap(
      Bundle.module.url(
        forResource: "app_attest_transcripts",
        withExtension: "json"
      ),
      "missing bundled gateway transcript fixture app_attest_transcripts.json in \(Bundle.module.bundleURL.path)"
    )
    let data = try Data(contentsOf: path)
    return try JSONDecoder().decode(Fixture.self, from: data)
  }

  private static func body(_ request: URLRequest) throws -> [String: Any] {
    let data: Data
    if let httpBody = request.httpBody {
      data = httpBody
    } else {
      let stream = try XCTUnwrap(request.httpBodyStream)
      stream.open()
      defer { stream.close() }
      var bytes = Data()
      var buffer = [UInt8](repeating: 0, count: 1_024)
      while true {
        let count = stream.read(&buffer, maxLength: buffer.count)
        if count < 0 {
          throw try XCTUnwrap(stream.streamError)
        }
        if count == 0 { break }
        bytes.append(buffer, count: count)
      }
      data = bytes
    }
    return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
  }

  private static func response(
    _ request: URLRequest,
    status: Int,
    json: [String: Any]
  ) -> (HTTPURLResponse, Data) {
    let data = try! JSONSerialization.data(withJSONObject: json, options: [.sortedKeys])
    let response = HTTPURLResponse(
      url: request.url!,
      statusCode: status,
      httpVersion: "HTTP/1.1",
      headerFields: ["Content-Type": "application/json"]
    )!
    return (response, data)
  }

  private static func hex<D: Sequence>(_ data: D) -> String where D.Element == UInt8 {
    data.map { String(format: "%02x", $0) }.joined()
  }
}

private final class MemoryGrantStore: BuzzPushEndpointGrantStore {
  var saved: [BuzzPushEndpointGrantRecord]
  var pending: [BuzzPushPendingEnrollmentRecord] = []
  var cleanup: [BuzzPushGatewayCleanupState] = []
  var replacementOrigins: [String] = []
  var replacementGeneration: Int64 = 0
  var legacyEndpointGrants: [String] = []
  var legacyPendingEnrollments: [BuzzPushLegacyRecoveryInventory.BuzzPushLegacyPendingRecovery] = []
  var resetOperations: [String] = []
  var grantSaveFailuresRemaining: Int
  var cleanupSaveFailureCalls: Set<Int>
  var pendingRemoveFailuresRemaining: Int
  private var cleanupSaveCallCount = 0
  init(
    records: [BuzzPushEndpointGrantRecord] = [],
    pending: [BuzzPushPendingEnrollmentRecord] = [],
    grantSaveFailuresRemaining: Int = 0,
    cleanupSaveFailureCalls: Set<Int> = [],
    pendingRemoveFailuresRemaining: Int = 0
  ) {
    saved = records
    self.pending = pending
    self.grantSaveFailuresRemaining = grantSaveFailuresRemaining
    self.cleanupSaveFailureCalls = cleanupSaveFailureCalls
    self.pendingRemoveFailuresRemaining = pendingRemoveFailuresRemaining
  }
  func reset(forGatewayOrigin gatewayOrigin: String) throws {
    try BuzzPushGatewayStateReset.run(
      gatewayOrigin: gatewayOrigin,
      records: saved,
      pendingEnrollments: pending,
      cleanupStates: cleanup,
      saveCleanupState: { state in
        self.resetOperations.append("cleanup:\(state.gatewayOrigin)")
        try self.saveGatewayCleanupState(state)
      },
      removeCleanupState: {
        self.resetOperations.append("cleanup-removed:\($0)")
        try self.removeGatewayCleanupState(gatewayOrigin: $0)
      },
      replaceRecords: {
        self.resetOperations.append("records")
        self.saved = $0
      },
      replacePendingEnrollments: {
        self.resetOperations.append("pending")
        self.pending = $0
      }
    )
  }
  func records() throws -> [BuzzPushEndpointGrantRecord] { saved }
  func save(_ record: BuzzPushEndpointGrantRecord) throws {
    if grantSaveFailuresRemaining > 0 {
      grantSaveFailuresRemaining -= 1
      throw NSError(domain: "MemoryGrantStore", code: 1)
    }
    saved.removeAll {
      $0.gatewayOrigin == record.gatewayOrigin && $0.relayOrigin == record.relayOrigin
        && $0.appProfile == record.appProfile
    }
    saved.append(record)
  }
  func removeRecord(gatewayOrigin: String, relayOrigin: String, appProfile: String) throws {
    saved.removeAll {
      $0.gatewayOrigin == gatewayOrigin && $0.relayOrigin == relayOrigin
        && $0.appProfile == appProfile
    }
  }
  func removeRecords(gatewayOrigin: String, installationHandle: String) throws {
    saved.removeAll {
      $0.gatewayOrigin == gatewayOrigin
        && $0.gatewayInstallationHandle == installationHandle
    }
  }
  func removeRecords(
    gatewayOrigin: String,
    installationHandle: String,
    relayPubkey: String
  ) throws {
    saved.removeAll {
      $0.gatewayOrigin == gatewayOrigin
        && $0.gatewayInstallationHandle == installationHandle
        && $0.relayPubkey == relayPubkey
    }
  }
  func pendingEnrollment(
    gatewayOrigin: String,
    relayOrigin: String,
    appProfile: String
  ) throws -> BuzzPushPendingEnrollmentRecord? {
    pending.first {
      $0.gatewayOrigin == gatewayOrigin && $0.relayOrigin == relayOrigin
        && $0.appProfile == appProfile
    }
  }
  func savePendingEnrollment(_ record: BuzzPushPendingEnrollmentRecord) throws {
    pending.removeAll {
      $0.gatewayOrigin == record.gatewayOrigin && $0.relayOrigin == record.relayOrigin
        && $0.appProfile == record.appProfile
    }
    pending.append(record)
  }
  func removePendingEnrollment(
    gatewayOrigin: String,
    relayOrigin: String,
    appProfile: String
  ) throws {
    if pendingRemoveFailuresRemaining > 0 {
      pendingRemoveFailuresRemaining -= 1
      throw NSError(domain: "MemoryGrantStore", code: 3)
    }
    pending.removeAll {
      $0.gatewayOrigin == gatewayOrigin && $0.relayOrigin == relayOrigin
        && $0.appProfile == appProfile
    }
  }
  func gatewayCleanupStates() throws -> [BuzzPushGatewayCleanupState] { cleanup }
  func saveGatewayCleanupState(_ state: BuzzPushGatewayCleanupState) throws {
    cleanupSaveCallCount += 1
    if cleanupSaveFailureCalls.contains(cleanupSaveCallCount) {
      throw NSError(domain: "MemoryGrantStore", code: 2)
    }
    cleanup.removeAll { $0.gatewayOrigin == state.gatewayOrigin }
    cleanup.append(state)
  }
  func removeGatewayCleanupState(gatewayOrigin: String) throws {
    cleanup.removeAll { $0.gatewayOrigin == gatewayOrigin }
  }
  func replacementQueueState() throws -> BuzzPushReplacementQueueState {
    BuzzPushReplacementQueueState(
      generation: replacementGeneration,
      relayOrigins: replacementOrigins
    )
  }
  func queueReplacementRelayOrigins(_ relayOrigins: [String]) throws {
    replacementGeneration += 1
    replacementOrigins = Array(Set(replacementOrigins + relayOrigins)).sorted()
  }
  func checkpointReplacementRelayOrigins(
    _ relayOrigins: [String],
    expectedGeneration: Int64
  ) throws -> Bool {
    guard replacementGeneration == expectedGeneration else { return false }
    let completedOrigins = Set(relayOrigins)
    replacementOrigins.removeAll { completedOrigins.contains($0) }
    return true
  }
  func clearReplacementRelayOrigins() throws {
    replacementGeneration += 1
    replacementOrigins = []
  }
  func quarantinedLegacyEndpointGrants() throws -> [String] { legacyEndpointGrants }
  func quarantinedLegacyPendingEnrollments() throws
    -> [BuzzPushLegacyRecoveryInventory.BuzzPushLegacyPendingRecovery]
  { legacyPendingEnrollments }
}

private final class RecordingAppAttest: BuzzDevAppAttesting {
  var clientData: [Data] = []
  var preparedAttestations: [BuzzDevAttestation] = []
  var assertionKeyIds: [String] = []

  func prepareAttestation() async throws -> BuzzDevAttestation {
    let prepared = BuzzDevAttestation(
      keyId: BuzzDevPushEnrollmentDriverTests.keyId,
      attestation: BuzzDevPushEnrollmentDriverTests.attestation
    )
    preparedAttestations.append(prepared)
    return prepared
  }

  func attestation(
    _ prepared: BuzzDevAttestation,
    clientData: Data
  ) async throws -> BuzzDevAttestation {
    self.clientData.append(clientData)
    return prepared
  }

  func assertion(keyId: String, clientData: Data) async throws -> String {
    self.clientData.append(clientData)
    assertionKeyIds.append(keyId)
    return BuzzDevPushEnrollmentDriverTests.assertion
  }
}

private final class MemoryAppAttestKeyIdStore: BuzzAppAttestKeyIdStoring {
  var keyIdValue: String?
  var savedKeyIds: [String] = []

  init(keyId: String? = nil) {
    keyIdValue = keyId
  }

  func keyId() throws -> String? { keyIdValue }

  func saveKeyId(_ keyId: String) throws {
    savedKeyIds.append(keyId)
    keyIdValue = keyId
  }
}

private final class RecordingDCAppAttestService: BuzzDCAppAttestServicing {
  let isSupported: Bool
  let generatedKeyId: String
  let attestationObject: Data
  let assertionObject: Data
  let error: Error?

  var generateKeyCallCount = 0
  var attestedKeyIds: [String] = []
  var attestationClientDataHashes: [Data] = []
  var assertedKeyIds: [String] = []
  var assertionClientDataHashes: [Data] = []

  init(
    isSupported: Bool = true,
    generatedKeyId: String = BuzzDevPushEnrollmentDriverTests.keyId,
    attestationObject: Data = Data("attestation-object".utf8),
    assertionObject: Data = Data("assertion-object".utf8),
    error: Error? = nil
  ) {
    self.isSupported = isSupported
    self.generatedKeyId = generatedKeyId
    self.attestationObject = attestationObject
    self.assertionObject = assertionObject
    self.error = error
  }

  func generateKey() async throws -> String {
    generateKeyCallCount += 1
    if let error { throw error }
    return generatedKeyId
  }

  func attestKey(_ keyId: String, clientDataHash: Data) async throws -> Data {
    attestedKeyIds.append(keyId)
    attestationClientDataHashes.append(clientDataHash)
    if let error { throw error }
    return attestationObject
  }

  func generateAssertion(_ keyId: String, clientDataHash: Data) async throws -> Data {
    assertedKeyIds.append(keyId)
    assertionClientDataHashes.append(clientDataHash)
    if let error { throw error }
    return assertionObject
  }
}

private final class URLProtocolStub: URLProtocol, @unchecked Sendable {
  static let lock = NSLock()
  static var handler: ((URLRequest) throws -> (HTTPURLResponse, Data))?
  static var requests: [URLRequest] = []

  override class func canInit(with request: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

  override func startLoading() {
    Self.lock.lock()
    Self.requests.append(request)
    let handler = Self.handler
    Self.lock.unlock()
    do {
      let (response, data) =
        try handler?(request)
        ?? {
          throw URLError(.unsupportedURL)
        }()
      client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
      client?.urlProtocol(self, didLoad: data)
      client?.urlProtocolDidFinishLoading(self)
    } catch {
      client?.urlProtocol(self, didFailWithError: error)
    }
  }

  override func stopLoading() {}

  static func reset() {
    lock.lock()
    handler = nil
    requests = []
    lock.unlock()
  }
}
