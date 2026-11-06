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
    let deviceNow = Self.now + 2
    let driver = try makeDriver(
      gatewayBaseURL: URL(string: "http://push.example:80/")!,
      store: store, appAttest: appAttest,
      now: { Date(timeIntervalSince1970: TimeInterval(deviceNow)) },
      lifetimeSeconds: Self.expiresAt - deviceNow
    )
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
        XCTAssertNil(body["app_profile"])
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
        XCTAssertEqual(body["v"] as? Int, 2)
        XCTAssertEqual(body["gateway_origin"] as? String, Self.gatewayOrigin)
        let requestExpiry = try XCTUnwrap(body["request_expires_at"] as? Int64)
        XCTAssertEqual(requestExpiry, deviceNow + 240)
        guard requestExpiry <= Self.now + 300 else {
          return Self.response(request, status: 400, json: ["error": "invalid_request_expiry"])
        }
        XCTAssertNil(body["challenge_id"])
        XCTAssertNil(body["challenge"])
        XCTAssertEqual(body["endpoint_epoch"] as? Int, 1)
        XCTAssertEqual(body["not_before"] as? Int64, deviceNow)
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
      expectedSHA256: "792966649266bfcebdb79de54e8ed3880746f58efd77427a0f43e88c650d79d0",
      fixture: makeFixtureTranscript(
        name: "enroll",
        replacements: []
      )
    )
    XCTAssertEqual(challengeCount, 1, "only enrollment uses the public challenge budget")
    XCTAssertEqual(
      appAttest.clientData[1],
      try BuzzPushTranscript.delegate(
        gatewayOrigin: Self.gatewayURL,
        requestExpiresAt: deviceNow + 240,
        installationHandle: UUID(uuidString: Self.installationHandle)!,
        endpointEpoch: 1, expectedGeneration: 0, generation: 1, relayPubkey: Self.relayPubkey,
        notBefore: deviceNow, expiresAt: Self.expiresAt
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
        endpointEpoch: 1,
        generation: 1,
        expiresAt: Self.expiresAt
      )
    )
    XCTAssertEqual(store.saved, [record])
  }

  func testLostGrantResponseReissuesAtNewerGenerationWhetherOrNotCommitted() async throws {
    for committed in [false, true] {
      let store = MemoryGrantStore()
      let appAttest = RecordingAppAttest()
      let driver = try makeDriver(store: store, appAttest: appAttest)
      var current: Int64 = 0
      var requests = 0
      var challenges = 0
      URLProtocolStub.handler = { request in
        switch request.url?.path {
        case "/":
          return Self.response(request, status: 200, json: ["push":["keys":[["pubkey":Self.relayPubkey,"current":true]]]])
        case "/v1/installations/challenges":
          challenges += 1
          return Self.response(request, status: 200, json: ["challenge_id":Self.firstChallengeId,"challenge":Self.challenge,"expires_at":Self.now+300])
        case "/v1/installations":
          return Self.response(request, status: 201, json: ["installation_handle":Self.installationHandle,"endpoint_epoch":1,"expires_at":Self.expiresAt])
        case "/v1/delegations":
          requests += 1
          let body = try Self.body(request)
          let generation = try XCTUnwrap(body["generation"] as? Int64)
          let expected = try XCTUnwrap(body["expected_generation"] as? Int64)
          XCTAssertEqual(store.pending.first?.delegationGeneration, generation)
          if requests == 1 {
            if committed { current = generation }
            throw URLError(.networkConnectionLost)
          }
          if expected != current {
            return Self.response(request, status: 409, json: ["error":"stale_generation","current_generation":current])
          }
          XCTAssertGreaterThan(generation, current)
          current = generation
          return Self.response(request, status: 201, json: ["endpoint_grant":"new-random-grant-\(generation)"])
        default:
          XCTFail("Unexpected request \(request.url!)")
          return Self.response(request, status: 500, json: [:])
        }
      }
      do {
        _ = try await driver.enroll(deviceToken: Data((1...32).map(UInt8.init)), relayURL: Self.relayURL)
        XCTFail("Expected lost grant response")
      } catch { XCTAssertEqual((error as NSError).domain, NSURLErrorDomain) }
      XCTAssertTrue(store.saved.isEmpty)
      XCTAssertEqual(store.pending.first?.delegationGeneration, 1)
      let record = try await driver.enroll(deviceToken: Data((1...32).map(UInt8.init)), relayURL: Self.relayURL)
      XCTAssertEqual(record.generation, committed ? 3 : 2)
      XCTAssertEqual(record.endpointGrant, "new-random-grant-\(record.generation)")
      XCTAssertEqual(challenges, 1)
      XCTAssertTrue(store.pending.isEmpty)
    }
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

    XCTAssertEqual(challengeCount, 1)
    XCTAssertEqual(installationCount, 2)
    XCTAssertEqual(recovered.endpointGrant, "opaque-grant")
    XCTAssertTrue(store.pending.isEmpty)
  }

  func testResponseLostEnrollmentSurvivesTokenRotationAndAnotherReplayFailure() async throws {
    let store = MemoryGrantStore()
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    let rotatedToken = Data(repeating: 42, count: 32)
    var oldRequests = 0
    var newRequests = 0
    var firstBody: [String: Any]?
    var delegatedHandles: [String] = []
    URLProtocolStub.handler = { request in
      switch (request.httpMethod, request.url?.absoluteString) {
      case ("GET", "https://relay.example/"):
        return Self.response(request, status: 200, json: [
          "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]]
        ])
      case ("POST", "http://push.example/v1/installations/challenges"):
        return Self.response(request, status: 200, json: [
          "challenge_id": Self.firstChallengeId,
          "challenge": Self.challenge,
          "expires_at": Self.now + 300,
        ])
      case ("POST", "http://push.example/v1/installations"):
        let body = try Self.body(request)
        let oldEndpoint = body["endpoint"] as? String == Self.endpoint
        if oldEndpoint {
          oldRequests += 1
          if let firstBody {
            XCTAssertTrue(NSDictionary(dictionary: body).isEqual(to: firstBody))
          } else {
            firstBody = body
          }
          if oldRequests <= 2 { throw URLError(.networkConnectionLost) }
        } else {
          newRequests += 1
          XCTAssertEqual(body["endpoint"] as? String, Self.hex(rotatedToken))
          XCTAssertEqual(delegatedHandles, [Self.installationHandle])
        }
        return Self.response(request, status: 201, json: [
          "installation_handle": Self.installationHandle,
          "endpoint_epoch": 1,
          "expires_at": Self.expiresAt,
        ])
      case ("POST", "http://push.example/v1/installations/endpoint"):
        newRequests += 1
        let body = try Self.body(request)
        XCTAssertEqual(body["endpoint"] as? String, Self.hex(rotatedToken))
        XCTAssertEqual(body["endpoint_epoch"] as? Int, 1)
        XCTAssertEqual(body["new_endpoint_epoch"] as? Int, 2)
        XCTAssertEqual(body["request_expires_at"] as? Int64, Self.now + 240)
        if newRequests == 1 { throw URLError(.networkConnectionLost) }
        return Self.response(request, status: 200, json: ["status":"rotated"])
      case ("POST", "http://push.example/v1/delegations"):
        let handle = try XCTUnwrap(Self.body(request)["installation_handle"] as? String)
        delegatedHandles.append(handle)
        return Self.response(request, status: 201, json: ["endpoint_grant": "grant-\(handle)"])
      default:
        XCTFail("Unexpected request, including any migration/revocation route: \(request.url!)")
        return Self.response(request, status: 500, json: [:])
      }
    }

    for token in [Data((1...32).map(UInt8.init)), rotatedToken] {
      do {
        _ = try await driver.enroll(deviceToken: token, relayURL: Self.relayURL)
        XCTFail("Expected response loss")
      } catch {
        XCTAssertEqual((error as NSError).domain, NSURLErrorDomain)
      }
      XCTAssertEqual(store.pending.first?.endpoint, Self.endpoint)
      XCTAssertTrue(store.saved.isEmpty)
    }
    do {
      _ = try await driver.enroll(deviceToken: rotatedToken, relayURL: Self.relayURL)
      XCTFail("Expected lost rotation response")
    } catch { XCTAssertEqual((error as NSError).domain, NSURLErrorDomain) }
    XCTAssertEqual(store.pending.first?.rotationFromEpoch, 1)
    XCTAssertEqual(store.pending.first?.endpointEpoch, 2)
    let grant = try await driver.enroll(deviceToken: rotatedToken, relayURL: Self.relayURL)
    XCTAssertEqual(oldRequests, 3)
    XCTAssertEqual(newRequests, 2)
    XCTAssertEqual(delegatedHandles, [Self.installationHandle, Self.installationHandle])
    XCTAssertEqual(grant.endpointEpoch, 2)
    XCTAssertEqual(grant.endpointHash, Self.hex(SHA256.hash(data: rotatedToken)))
    XCTAssertTrue(store.pending.isEmpty)
  }

  func testRejectedEnrollmentReplayIsReplacedOnlyOnTerminalStatus() async throws {
    // A journaled request the gateway rejects must not be replayed for its whole
    // lifetime, nor replaced (with a fresh App Attest key) on every retry; a
    // transient failure, or a 400 that a later policy change could lift, must
    // keep the journal. A fresh status of 0 drops the replacement request, whose
    // replay then finds its challenge expired: an ordinary loss, not a
    // rejection. A fresh 409 conflicts with a live installation for the same
    // token, for example one whose request committed before its response was
    // lost and was rejected once its attestation certificate expired; its
    // consumed challenge then returns 404, as does a fresh 404's replay.
    let interval = BuzzDevPushEnrollmentDriver.rejectedEnrollmentReplacementSeconds
    let cases: [(replay: Int, fresh: Int?, elapsed: Int64, legacy: Bool, keys: Int)] = [
      (400, nil, interval, false, 1), (401, 201, interval, false, 2),
      (404, 201, 0, false, 2), (404, 0, 0, false, 3), (404, 404, 0, false, 2),
      (404, 409, 0, false, 2), (401, 201, 0, true, 2),
      (401, nil, interval - 1, false, 1), (401, 401, interval, false, 2),
      (401, 409, interval, false, 2), (503, nil, interval, false, 1),
    ]
    for (replayStatus, freshStatus, elapsed, legacy, keys) in cases {
      let fresh = freshStatus.map { $0 == 0 ? "dropped" : String($0) } ?? "none"
      let label = "replay \(replayStatus), fresh \(fresh) after \(elapsed)s, legacy \(legacy)"
      // Status for a later exact replay of a fresh request.
      let freshReplayStatus = [0, 409].contains(freshStatus) ? 404 : freshStatus
      let store = MemoryGrantStore()
      let appAttest = RecordingAppAttest()
      var clock = Self.now
      let driver = try makeDriver(
        store: store,
        appAttest: appAttest,
        now: { Date(timeIntervalSince1970: TimeInterval(clock)) },
        lifetimeSeconds: 30 * 86_400
      )
      var challengeCount = 0
      var replayRequests = 0
      var sentChallenges = Set<String>()
      URLProtocolStub.handler = { request in
        switch (request.httpMethod, request.url?.absoluteString) {
        case ("GET", "https://relay.example/"):
          return Self.response(request, status: 200, json: [
            "push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]]
          ])
        case ("POST", "http://push.example/v1/installations/challenges"):
          challengeCount += 1
          return Self.response(request, status: 200, json: [
            "challenge_id": [Self.firstChallengeId, Self.secondChallengeId].indices
              .contains(challengeCount - 1)
              ? [Self.firstChallengeId, Self.secondChallengeId][challengeCount - 1]
              : UUID().uuidString.lowercased(),
            "challenge": Self.challenge,
            "expires_at": clock + 300,
          ])
        case ("POST", "http://push.example/v1/installations"):
          let body = try Self.body(request)
          if body["challenge_id"] as? String == Self.firstChallengeId {
            replayRequests += 1
            if replayRequests == 1 { throw URLError(.networkConnectionLost) }
            return Self.response(request, status: replayStatus, json: ["error": "rejected"])
          }
          let challengeId = try XCTUnwrap(body["challenge_id"] as? String)
          let isReplay = !sentChallenges.insert(challengeId).inserted
          let status = try XCTUnwrap(
            isReplay ? freshReplayStatus : freshStatus,
            "\(label) must keep the journal"
          )
          if status == 0 { throw URLError(.networkConnectionLost) }
          guard status == 201 else {
            return Self.response(request, status: status, json: ["error": "rejected"])
          }
          return Self.response(request, status: 201, json: [
            "installation_handle": Self.installationHandle,
            "endpoint_epoch": 1,
            "expires_at": try XCTUnwrap(body["expires_at"] as? Int64),
          ])
        case ("POST", "http://push.example/v1/delegations"):
          return Self.response(request, status: 201, json: ["endpoint_grant": "opaque-grant"])
        default:
          XCTFail("Unexpected request \(request.url?.absoluteString ?? "nil")")
          return Self.response(request, status: 500, json: [:])
        }
      }
      let token = Data((1...32).map(UInt8.init))

      do {
        _ = try await driver.enroll(deviceToken: token, relayURL: Self.relayURL)
        XCTFail("Expected the simulated lost installation response")
      } catch {
        XCTAssertEqual((error as NSError).domain, NSURLErrorDomain)
      }
      let journal = try XCTUnwrap(store.pending.first)
      XCTAssertEqual(journal.preparedAt, Self.now)
      if legacy {
        store.pending = [Self.withoutPreparedAt(journal)]
      }
      clock += elapsed

      // Two retries: the second must not replace a journal again.
      for attempt in 0..<2 {
        do {
          let grant = try await driver.enroll(deviceToken: token, relayURL: Self.relayURL)
          XCTAssertEqual(freshStatus, 201, label)
          XCTAssertEqual(grant.endpointGrant, "opaque-grant")
          XCTAssertTrue(store.pending.isEmpty)
        } catch BuzzDevPushEnrollmentError.unexpectedStatus(
          route: "v1/installations", _, let actual, _
        ) {
          // The failure stays visible and leaves a durable journal to retry.
          XCTAssertEqual(
            actual,
            (attempt == 0 ? freshStatus : freshReplayStatus) ?? replayStatus,
            label
          )
          let challengeId = try XCTUnwrap(store.pending.first?.challengeId, label)
          if freshStatus == nil {
            XCTAssertEqual(challengeId, Self.firstChallengeId, label)
          } else {
            XCTAssertNotEqual(challengeId, Self.firstChallengeId, label)
          }
        } catch let error as URLError where freshStatus == 0 {
          // Each retry replaces the lost replacement at once and loses it again.
          XCTAssertEqual(error.code, .networkConnectionLost, label)
          XCTAssertNotEqual(store.pending.first?.challengeId, Self.firstChallengeId, label)
        }
      }
      XCTAssertEqual(appAttest.preparedAttestations.count, keys, label)
      if freshStatus == 0 {
        XCTAssertEqual(store.pending.first?.rejectedReplacements ?? 0, 0, label)
        // An ordinary replacement that fails after generating its key still
        // records the attempt, so the next retry waits instead of generating
        // another key, however old the journal was.
        clock += 4 * interval
        appAttest.failAttestation = true
        do {
          _ = try await driver.enroll(deviceToken: token, relayURL: Self.relayURL)
          XCTFail("Expected the simulated attestation failure: \(label)")
        } catch {
          XCTAssertEqual((error as NSError).domain, NSURLErrorDomain, label)
        }
        await Self.assertEnrollmentStatus(driver, token, 404, label)
        XCTAssertEqual(appAttest.preparedAttestations.count, keys + 1, label)
      }

      guard let freshStatus, let freshReplayStatus, [401, 404, 409].contains(freshStatus) else {
        continue
      }
      // Each consecutive replacement doubles the wait before the next one.
      XCTAssertEqual(store.pending.first?.rejectedReplacements, 1, label)
      clock += interval
      await Self.assertEnrollmentStatus(driver, token, freshReplayStatus, label)
      XCTAssertEqual(appAttest.preparedAttestations.count, 2, label)

      // A replacement that fails after generating its key keeps the rejected
      // journal with the attempt recorded, so retries wait instead of
      // generating more keys, even across a driver restart.
      clock += interval
      appAttest.failAttestation = true
      do {
        _ = try await driver.enroll(deviceToken: token, relayURL: Self.relayURL)
        XCTFail("Expected the simulated attestation failure: \(label)")
      } catch {
        XCTAssertEqual((error as NSError).domain, NSURLErrorDomain, label)
      }
      XCTAssertEqual(appAttest.preparedAttestations.count, 3, label)
      XCTAssertEqual(store.pending.first?.challengeId, Self.secondChallengeId, label)
      XCTAssertEqual(store.pending.first?.rejectedReplacements, 2, label)
      let restarted = try makeDriver(
        store: store,
        appAttest: appAttest,
        now: { Date(timeIntervalSince1970: TimeInterval(clock)) },
        lifetimeSeconds: 30 * 86_400
      )
      await Self.assertEnrollmentStatus(restarted, token, freshReplayStatus, label)
      XCTAssertEqual(appAttest.preparedAttestations.count, 3, label)
      clock += 4 * interval
      await Self.assertEnrollmentStatus(restarted, token, freshStatus, label)
      XCTAssertEqual(appAttest.preparedAttestations.count, 4, label)
      XCTAssertEqual(store.pending.first?.rejectedReplacements, 3, label)
    }
  }

  private static func assertEnrollmentStatus(
    _ driver: BuzzDevPushEnrollmentDriver,
    _ token: Data,
    _ status: Int,
    _ label: String
  ) async {
    do {
      _ = try await driver.enroll(deviceToken: token, relayURL: relayURL)
      XCTFail("Expected gateway status \(status): \(label)")
    } catch BuzzDevPushEnrollmentError.unexpectedStatus(
      route: "v1/installations", _, let actual, _
    ) {
      XCTAssertEqual(actual, status, label)
    } catch {
      XCTFail("Unexpected error \(error): \(label)")
    }
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
      #"{"relayOrigin":"wss://relay.example","relayPubkey":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","installationId":"000102030405060708090a0b0c0d0e0f","endpointGrant":"opaque","endpointHash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","endpointEpoch":1,"generation":1,"expiresAt":1752624000}"#
        .utf8
    )

    XCTAssertThrowsError(try JSONDecoder().decode(BuzzPushEndpointGrantRecord.self, from: data))
  }

  func testGrantWithoutAppAttestKeyIsRejected() throws {
    let data = Data(
      #"{"gatewayOrigin":"https://push.example","relayOrigin":"wss://relay.example","relayPubkey":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","installationId":"000102030405060708090a0b0c0d0e0f","endpointGrant":"opaque","endpointHash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","endpointEpoch":1,"generation":1,"expiresAt":1752624000}"#
        .utf8
    )

    XCTAssertThrowsError(try JSONDecoder().decode(BuzzPushEndpointGrantRecord.self, from: data))
  }

  func testOtherGatewayGrantIsNeitherExportedNorMigrated() throws {
    let other = BuzzPushEndpointGrantRecord(
      gatewayOrigin: "https://other-push.example",
      relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey,
      appAttestKeyId: Self.keyId,
      installationId: Self.installationId,
      endpointGrant: "other-gateway-grant",
      endpointHash: Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init)))),
      endpointEpoch: 1,
      generation: 1,
      expiresAt: Self.expiresAt
    )
    let store = MemoryGrantStore(records: [other])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())

    XCTAssertEqual(try driver.endpointGrants(), [])
    XCTAssertEqual(store.saved, [other])
    XCTAssertTrue(URLProtocolStub.requests.isEmpty)
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

  func testDirectRevocationRetriesWithoutChallengeAndPreventsLocalGrantReuse() async throws {
    let store = MemoryGrantStore(records: [BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin, relayOrigin: "wss://relay.example",
      relayPubkey: Self.relayPubkey, gatewayInstallationHandle: Self.installationHandle,
      appAttestKeyId: Self.keyId, installationId: Self.installationId,
      endpointGrant: "old-opaque-grant", endpointHash: Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init)))),
      endpointEpoch: 1, generation: 1, expiresAt: Self.expiresAt
    )])
    let appAttest = RecordingAppAttest()
    let driver = try makeDriver(store: store, appAttest: appAttest)
    var requests = 0
    URLProtocolStub.handler = { request in
      XCTAssertEqual(request.url?.path, "/v1/delegations/revoke")
      let body = try Self.body(request)
      XCTAssertEqual(body["v"] as? Int, 2)
      XCTAssertNil(body["challenge"])
      XCTAssertEqual(body["request_expires_at"] as? Int64, Self.now + 240)
      XCTAssertEqual(body["generation"] as? Int, 1)
      XCTAssertEqual(store.saved.first?.endpointGrant, "")
      requests += 1
      if requests == 1 { throw URLError(.networkConnectionLost) }
      return Self.response(request, status: 200, json: ["status":"revoked"])
    }
    do {
      try await driver.revokeDelegation(installationHandle: UUID(uuidString: Self.installationHandle)!, keyId: Self.keyId, relayPubkey: Self.relayPubkey, generation: 1)
      XCTFail("Expected lost revocation response")
    } catch { XCTAssertEqual((error as NSError).domain, NSURLErrorDomain) }
    try await driver.revokeDelegation(installationHandle: UUID(uuidString: Self.installationHandle)!, keyId: Self.keyId, relayPubkey: Self.relayPubkey, generation: 1)
    XCTAssertEqual(requests, 2)
    XCTAssertEqual(store.saved.first?.endpointGrant, "")
    XCTAssertEqual(store.saved.first?.generation, 1)
    XCTAssertEqual(store.saved.first?.expiresAt, Self.expiresAt)
  }

  func testCanceledFirstEnrollmentSettlesLostInstallationResponseThenRevokes() async throws {
    let store = MemoryGrantStore(pending: [BuzzPushPendingEnrollmentRecord(
      gatewayOrigin: Self.gatewayOrigin, relayOrigin: "wss://relay.example", relayPubkey: Self.relayPubkey,
      endpoint: Self.endpoint, endpointHash: Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init)))),
      expiresAt: Self.expiresAt, installationId: Self.installationId,
      challengeId: Self.firstChallengeId, challenge: Self.challenge, keyId: Self.keyId, attestation: Self.attestation)])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    var paths: [String] = []
    URLProtocolStub.handler = { request in
      let path = request.url!.path
      paths.append(path)
      switch path {
      case "/v1/installations":
        return Self.response(request, status: 201, json: ["installation_handle":Self.installationHandle,"endpoint_epoch":1,"expires_at":Self.expiresAt])
      case "/v1/delegations":
        return Self.response(request, status: 201, json: ["endpoint_grant":"reissued-grant"])
      case "/v1/delegations/revoke":
        XCTAssertEqual(try Self.body(request)["generation"] as? Int, 1)
        return Self.response(request, status: 200, json: ["status":"revoked"])
      default:
        XCTFail("Unexpected request \(path)")
        return Self.response(request, status: 500, json: [:])
      }
    }
    try await driver.revokePendingDelegation(relayOrigin: "wss://relay.example", relayPubkey: "")
    XCTAssertEqual(paths, ["/v1/installations", "/v1/delegations", "/v1/delegations/revoke"])
    XCTAssertTrue(store.pending.isEmpty)
    XCTAssertEqual(store.saved.first?.endpointGrant, "")
  }

  func testCancellationSettlesLostRenewalAndRevokesItsNewerAuthority() async throws {
    let endpointHash = Self.hex(SHA256.hash(data: Data((1...32).map(UInt8.init))))
    let store = MemoryGrantStore(records: [BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin, relayOrigin: "wss://relay.example", relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle, appAttestKeyId: Self.keyId,
      installationId: Self.installationId, endpointGrant: "old-grant", endpointHash: endpointHash,
      endpointEpoch: 1, generation: 1, expiresAt: Self.now + 10
    )], pending: [BuzzPushPendingEnrollmentRecord(
      gatewayOrigin: Self.gatewayOrigin, relayOrigin: "wss://relay.example", relayPubkey: Self.relayPubkey,
      endpoint: Self.endpoint, endpointHash: endpointHash, expiresAt: Self.expiresAt,
      installationId: Self.installationId, gatewayInstallationHandle: Self.installationHandle,
      keyId: Self.keyId, delegationGeneration: 2, expectedDelegationGeneration: 1
    )])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    var current: Int64 = 2
    var revoked: [Int64] = []
    URLProtocolStub.handler = { request in
      let body = try Self.body(request)
      switch request.url?.path {
      case "/v1/delegations":
        let expected = try XCTUnwrap(body["expected_generation"] as? Int64)
        if expected != current {
          return Self.response(request, status: 409, json: ["error":"stale_generation","current_generation":current])
        }
        current = try XCTUnwrap(body["generation"] as? Int64)
        return Self.response(request, status: 201, json: ["endpoint_grant":"replacement-grant"])
      case "/v1/delegations/revoke":
        let generation = try XCTUnwrap(body["generation"] as? Int64)
        XCTAssertEqual(generation, current)
        revoked.append(generation)
        return Self.response(request, status: 200, json: ["status":"revoked"])
      default:
        XCTFail("Unexpected request \(request.url!)")
        return Self.response(request, status: 500, json: [:])
      }
    }
    try await driver.revokeDelegation(installationHandle: UUID(uuidString: Self.installationHandle)!,
      keyId: Self.keyId, relayPubkey: Self.relayPubkey, generation: 1, settlePending: true)
    XCTAssertEqual(revoked, [4])
    XCTAssertTrue(store.pending.isEmpty)
    XCTAssertEqual(store.saved.first?.generation, 4)
    XCTAssertEqual(store.saved.first?.endpointGrant, "")
    XCTAssertEqual(store.saved.first?.expiresAt, Self.expiresAt)
  }

  func testReplacementRetiresExactPriorAuthorityBeforeLosingItsRecord() async throws {
    let token = Data((1...32).map(UInt8.init))
    let endpointHash = Self.hex(SHA256.hash(data: token))
    let currentHandle = "44444444-4444-4444-8444-444444444444"
    let currentKeyId = Data(repeating: 0xBB, count: 32).base64EncodedString()
    for (oldEpoch, outcome) in [
      (Int64(1), "success"), (3, "success"), (1, "revocation failure"),
      (1, "revocation response loss"), (1, "issuance response loss"),
      (1, "shared"), (1, "cached"), (1, "cached revocation failure"),
      (1, "expired cleanup failure"), (1, "expired enrollment failure"),
      (1, "superseded cleanup failure"), (1, "superseded enrollment failure"),
      (1, "expired shared cleanup failure"),
    ] {
      let discardingJournal = outcome.hasPrefix("expired") || outcome.hasPrefix("superseded")
      let cached = outcome.hasPrefix("cached") || outcome == "shared"
      let old = BuzzPushEndpointGrantRecord(
        gatewayOrigin: Self.gatewayOrigin, relayOrigin: "wss://relay.example",
        relayPubkey: discardingJournal ? String(repeating: "c", count: 64) : Self.relayPubkey,
        gatewayInstallationHandle: Self.installationHandle,
        appAttestKeyId: Self.keyId, installationId: Self.installationId,
        endpointGrant: "old-token-grant", endpointHash: "old-token-hash",
        endpointEpoch: oldEpoch, generation: 7,
        expiresAt: discardingJournal ? Self.now + 3600 : Self.expiresAt)
      let current = BuzzPushEndpointGrantRecord(
        gatewayOrigin: Self.gatewayOrigin, relayOrigin: "wss://other.example",
        relayPubkey: cached ? Self.relayPubkey : String(repeating: "b", count: 64),
        gatewayInstallationHandle: currentHandle,
        appAttestKeyId: currentKeyId, installationId: String(repeating: "f", count: 32),
        endpointGrant: "current-token-grant", endpointHash: endpointHash,
        endpointEpoch: 1, generation: 1,
        expiresAt: discardingJournal ? Self.now + 360 : Self.expiresAt)
      let shared = BuzzPushEndpointGrantRecord(
        gatewayOrigin: old.gatewayOrigin, relayOrigin: "wss://shared.example",
        relayPubkey: old.relayPubkey, gatewayInstallationHandle: old.gatewayInstallationHandle,
        appAttestKeyId: old.appAttestKeyId, installationId: String(repeating: "e", count: 32),
        endpointGrant: old.endpointGrant, endpointHash: old.endpointHash,
        endpointEpoch: old.endpointEpoch, generation: old.generation, expiresAt: old.expiresAt)
      // MemoryGrantStore.save matches Keychain's gateway+origin replacement,
      // so the old target is lost if retirement happens after save.
      var store = MemoryGrantStore(records: [old, current] + (outcome == "shared" ? [shared] : []))
      let appAttest = RecordingAppAttest()
      var clock = Self.now
      var driver = try makeDriver(store: store, appAttest: appAttest,
        now: { Date(timeIntervalSince1970: TimeInterval(clock)) })
      var generation: Int64 = cached ? 1 : 0
      var revocations = 0
      URLProtocolStub.handler = { request in
        switch (request.httpMethod, request.url?.path) {
        case ("GET", "/"):
          return Self.response(request, status: 200,
            json: ["push": ["keys": [["pubkey": Self.relayPubkey, "current": true]]]])
        case ("POST", "/v1/delegations"):
          XCTAssertTrue(!cached || outcome == "shared")
          let body = try Self.body(request)
          XCTAssertEqual(body["installation_handle"] as? String, currentHandle)
          XCTAssertEqual(body["endpoint_epoch"] as? Int, 1)
          XCTAssertEqual(store.pending.first?.gatewayInstallationHandle, currentHandle)
          XCTAssertNil(store.pending.first?.rotationFromEpoch)
          let expected = try XCTUnwrap(body["expected_generation"] as? Int64)
          if expected != generation {
            return Self.response(request, status: 409,
              json: ["error": "stale_generation", "current_generation": generation])
          }
          let next = try XCTUnwrap(body["generation"] as? Int64)
          XCTAssertGreaterThan(next, generation)
          generation = next
          if outcome == "issuance response loss" && generation == 1 {
            throw URLError(.networkConnectionLost)
          }
          return Self.response(request, status: 201, json: ["endpoint_grant": "renewed-grant"])
        case ("POST", "/v1/delegations/revoke"):
          revocations += 1
          let body = try Self.body(request)
          XCTAssertEqual(body["installation_handle"] as? String, old.gatewayInstallationHandle)
          XCTAssertEqual(body["relay_pubkey"] as? String, old.relayPubkey)
          XCTAssertEqual(body["generation"] as? Int64, old.generation)
          XCTAssertEqual(appAttest.assertionKeyIds.last, old.appAttestKeyId)
          let retained = try XCTUnwrap(store.saved.first {
            $0.gatewayInstallationHandle == old.gatewayInstallationHandle && $0.relayPubkey == old.relayPubkey
          })
          XCTAssertEqual(retained.gatewayInstallationHandle, old.gatewayInstallationHandle)
          XCTAssertEqual(retained.generation, old.generation)
          XCTAssertEqual(retained.endpointGrant, "")
          if outcome != "shared" { XCTAssertEqual(store.pending.isEmpty, cached) }
          if revocations <= (discardingJournal ? 2 : 1)
            && (discardingJournal || outcome.contains("revocation failure"))
          {
            return Self.response(request, status: 503, json: [:])
          }
          if revocations == 1 && outcome == "revocation response loss" {
            throw URLError(.networkConnectionLost)
          }
          return Self.response(request, status: 200, json: ["status": "revoked"])
        default:
          XCTFail("The current token already has an installation; must not rotate or enroll")
          return Self.response(request, status: 409, json: ["error": "endpoint_in_use"])
        }
      }

      if outcome.contains("failure") || outcome.contains("loss") {
        do {
          _ = try await driver.enroll(deviceToken: token, relayURL: Self.relayURL)
          XCTFail("Expected failure before replacing the old target: \(outcome)")
        } catch {
          XCTAssertEqual(store.saved.first?.gatewayInstallationHandle, old.gatewayInstallationHandle)
          XCTAssertEqual(store.pending.isEmpty, cached)
        }
        // Restart from only Codable durable state, including the old target and
        // the new issuance journal. A committed response loss needs a fresh grant.
        store = MemoryGrantStore(
          records: try JSONDecoder().decode([BuzzPushEndpointGrantRecord].self,
            from: JSONEncoder().encode(store.saved)),
          pending: try JSONDecoder().decode([BuzzPushPendingEnrollmentRecord].self,
            from: JSONEncoder().encode(store.pending)))
        driver = try makeDriver(store: store, appAttest: appAttest,
          now: { Date(timeIntervalSince1970: TimeInterval(clock)) })
      }
      if discardingJournal {
        let journal = try XCTUnwrap(store.pending.first)
        XCTAssertEqual(journal.expiresAt, Self.now + 360)
        XCTAssertNotEqual(journal.relayPubkey, old.relayPubkey)
        if outcome.hasPrefix("expired") {
          clock = Self.now + 361
        } else {
          try store.save(BuzzPushEndpointGrantRecord(
            gatewayOrigin: current.gatewayOrigin, relayOrigin: current.relayOrigin,
            relayPubkey: current.relayPubkey, gatewayInstallationHandle: currentHandle,
            appAttestKeyId: currentKeyId, installationId: current.installationId,
            endpointGrant: current.endpointGrant, endpointHash: current.endpointHash,
            endpointEpoch: 2, generation: current.generation, expiresAt: current.expiresAt))
        }
        XCTAssertGreaterThan(old.expiresAt, clock)
        if outcome.contains("shared") {
          try store.save(shared)
          try await driver.revokePendingDelegation(relayOrigin: old.relayOrigin,
            relayPubkey: journal.relayPubkey)
          XCTAssertEqual(revocations, 1, "Only a positive old owner permits skipping retirement")
          XCTAssertTrue(store.saved.contains(shared))
        } else {
          do {
            if outcome.contains("enrollment") {
              _ = try await driver.enroll(deviceToken: token, relayURL: Self.relayURL)
            } else {
              try await driver.revokePendingDelegation(relayOrigin: old.relayOrigin,
                relayPubkey: journal.relayPubkey)
            }
            XCTFail("Failed retirement must retain the replacement journal")
          } catch BuzzDevPushEnrollmentError.unexpectedStatus(
            route: "v1/delegations/revoke", _, 503, _
          ) {}
          XCTAssertEqual(revocations, 2)
          XCTAssertEqual(store.pending, [journal])
          XCTAssertEqual(store.saved.first?.gatewayInstallationHandle, old.gatewayInstallationHandle)
          XCTAssertEqual(store.saved.first?.relayPubkey, old.relayPubkey)
          XCTAssertEqual(store.saved.first?.endpointGrant, "")
          try await driver.revokePendingDelegation(relayOrigin: old.relayOrigin,
            relayPubkey: journal.relayPubkey)
          XCTAssertEqual(revocations, 3)
        }
        XCTAssertTrue(store.pending.isEmpty)
        continue
      }
      let record = try await driver.enroll(deviceToken: token, relayURL: Self.relayURL)

      XCTAssertEqual(record.gatewayInstallationHandle, currentHandle)
      XCTAssertEqual(record.installationId, old.installationId)
      XCTAssertEqual(record.endpointEpoch, 1)
      XCTAssertEqual(record.endpointHash, endpointHash)
      XCTAssertEqual(record.endpointGrant, cached ? current.endpointGrant : "renewed-grant")
      XCTAssertEqual(revocations, outcome == "shared" ? 0 : (outcome.contains("revocation") ? 2 : 1))
      XCTAssertEqual(store.saved.first { $0.relayOrigin == old.relayOrigin }, record)
      XCTAssertFalse(store.saved.contains(old))
      XCTAssertTrue(store.pending.isEmpty)
      XCTAssertTrue(store.saved.contains(current))
      if outcome == "shared" {
        XCTAssertTrue(store.saved.contains(shared))
        // Removing the last old origin must leave the new installation and its
        // same-signer pending renewal untouched.
        let unrelatedPending = BuzzPushPendingEnrollmentRecord(
          gatewayOrigin: Self.gatewayOrigin, relayOrigin: record.relayOrigin,
          relayPubkey: record.relayPubkey, endpoint: Self.endpoint, endpointHash: endpointHash,
          endpointEpoch: 1, expiresAt: Self.expiresAt, installationId: record.installationId,
          gatewayInstallationHandle: currentHandle, keyId: currentKeyId,
          delegationGeneration: 2, expectedDelegationGeneration: record.generation)
        try store.savePendingEnrollment(unrelatedPending)
        try await driver.revokeDelegation(installationHandle: UUID(uuidString: Self.installationHandle)!,
          keyId: old.appAttestKeyId, relayPubkey: old.relayPubkey, generation: old.generation,
          settlePending: true)
        XCTAssertEqual(revocations, 1)
        XCTAssertTrue(store.saved.contains(record))
        XCTAssertTrue(store.saved.contains(current))
        XCTAssertEqual(store.pending, [unrelatedPending])
        try await driver.revokePendingDelegation(relayOrigin: record.relayOrigin,
          relayPubkey: record.relayPubkey)
        XCTAssertEqual(revocations, 1, "A recovered shared installation must not be revoked")
        XCTAssertTrue(store.saved.contains(current))
        XCTAssertTrue(store.pending.isEmpty)
        let recovered = try XCTUnwrap(store.saved.first { $0.relayOrigin == record.relayOrigin })
        XCTAssertEqual(recovered.gatewayInstallationHandle, currentHandle)
        XCTAssertGreaterThan(recovered.generation, record.generation)
        XCTAssertFalse(recovered.endpointGrant.isEmpty)
      }
    }
  }

  func testSiblingRotationFencesAnOlderPendingRotation() async throws {
    let token = Data(repeating: 43, count: 32)
    let endpointHash = Self.hex(SHA256.hash(data: token))
    let old = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin, relayOrigin: "wss://relay.example", relayPubkey: Self.relayPubkey,
      gatewayInstallationHandle: Self.installationHandle, appAttestKeyId: Self.keyId,
      installationId: Self.installationId, endpointGrant: "old-grant", endpointHash: "old-hash",
      endpointEpoch: 1, generation: 1, expiresAt: Self.expiresAt)
    let sibling = BuzzPushEndpointGrantRecord(
      gatewayOrigin: Self.gatewayOrigin, relayOrigin: "wss://other.example", relayPubkey: String(repeating: "b", count: 64),
      gatewayInstallationHandle: Self.installationHandle, appAttestKeyId: Self.keyId,
      installationId: String(repeating: "1", count: 32), endpointGrant: "current-sibling", endpointHash: endpointHash,
      endpointEpoch: 3, generation: 2, expiresAt: Self.expiresAt)
    let pending = BuzzPushPendingEnrollmentRecord(
      gatewayOrigin: Self.gatewayOrigin, relayOrigin: "wss://relay.example", relayPubkey: Self.relayPubkey,
      endpoint: Self.hex(Data(repeating: 42, count: 32)), endpointHash: Self.hex(SHA256.hash(data: Data(repeating: 42, count: 32))),
      endpointEpoch: 2, rotationFromEpoch: 1, expiresAt: Self.expiresAt,
      installationId: Self.installationId, gatewayInstallationHandle: Self.installationHandle, keyId: Self.keyId)
    let store = MemoryGrantStore(records: [old, sibling], pending: [pending])
    let driver = try makeDriver(store: store, appAttest: RecordingAppAttest())
    URLProtocolStub.handler = { request in
      if request.httpMethod == "GET" {
        return Self.response(request, status: 200, json: ["push":["keys":[["pubkey":Self.relayPubkey,"current":true]]]])
      }
      XCTAssertEqual(request.url?.path, "/v1/delegations", "an old rotation must never be replayed")
      XCTAssertEqual(try Self.body(request)["endpoint_epoch"] as? Int, 3)
      return Self.response(request, status: 201, json: ["endpoint_grant":"current-grant"])
    }
    let record = try await driver.enroll(deviceToken: token, relayURL: Self.relayURL)
    XCTAssertEqual(record.endpointEpoch, 3)
    XCTAssertEqual(record.endpointHash, endpointHash)
    XCTAssertTrue(store.pending.isEmpty)
  }

  private func makeDriver(
    gatewayBaseURL: URL = BuzzDevPushEnrollmentDriverTests.gatewayURL,
    store: BuzzPushEndpointGrantStore,
    appAttest: BuzzDevAppAttesting,
    installationIdBytes: @escaping () throws -> Data = {
      Data(0..<16)
    },
    now: @escaping () -> Date = {
      Date(timeIntervalSince1970: TimeInterval(BuzzDevPushEnrollmentDriverTests.now))
    },
    lifetimeSeconds: Int64 = BuzzDevPushEnrollmentDriverTests.expiresAt
      - BuzzDevPushEnrollmentDriverTests.now
  ) throws -> BuzzDevPushEnrollmentDriver {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [URLProtocolStub.self]
    return try BuzzDevPushEnrollmentDriver(
      gatewayBaseURL: gatewayBaseURL,
      store: store,
      session: URLSession(configuration: configuration),
      appAttest: appAttest,
      now: now,
      lifetimeSeconds: lifetimeSeconds,
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

  private static func withoutPreparedAt(
    _ journal: BuzzPushPendingEnrollmentRecord
  ) -> BuzzPushPendingEnrollmentRecord {
    BuzzPushPendingEnrollmentRecord(
      gatewayOrigin: journal.gatewayOrigin,
      relayOrigin: journal.relayOrigin,
      relayPubkey: journal.relayPubkey,
      endpoint: journal.endpoint,
      endpointHash: journal.endpointHash,
      endpointEpoch: journal.endpointEpoch,
      rotationFromEpoch: journal.rotationFromEpoch,
      expiresAt: journal.expiresAt,
      installationId: journal.installationId,
      gatewayInstallationHandle: journal.gatewayInstallationHandle,
      challengeId: journal.challengeId,
      challenge: journal.challenge,
      keyId: journal.keyId,
      attestation: journal.attestation,
      delegationGeneration: journal.delegationGeneration,
      expectedDelegationGeneration: journal.expectedDelegationGeneration
    )
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
  var grantSaveFailuresRemaining: Int
  var pendingRemoveFailuresRemaining: Int
  init(
    records: [BuzzPushEndpointGrantRecord] = [],
    pending: [BuzzPushPendingEnrollmentRecord] = [],
    grantSaveFailuresRemaining: Int = 0,
    pendingRemoveFailuresRemaining: Int = 0
  ) {
    saved = records
    self.pending = pending
    self.grantSaveFailuresRemaining = grantSaveFailuresRemaining
    self.pendingRemoveFailuresRemaining = pendingRemoveFailuresRemaining
  }
  func records() throws -> [BuzzPushEndpointGrantRecord] { saved }
  func pendingEnrollments() throws -> [BuzzPushPendingEnrollmentRecord] { pending }
  func replaceAll(_ records: [BuzzPushEndpointGrantRecord]) throws { saved = records }
  func save(_ record: BuzzPushEndpointGrantRecord) throws {
    if grantSaveFailuresRemaining > 0 {
      grantSaveFailuresRemaining -= 1
      throw NSError(domain: "MemoryGrantStore", code: 1)
    }
    saved.removeAll {
      $0.gatewayOrigin == record.gatewayOrigin && $0.relayOrigin == record.relayOrigin
    }
    saved.append(record)
  }
  func pendingEnrollment(
    gatewayOrigin: String,
    relayOrigin: String
  ) throws -> BuzzPushPendingEnrollmentRecord? {
    pending.first {
      $0.gatewayOrigin == gatewayOrigin && $0.relayOrigin == relayOrigin
    }
  }
  func savePendingEnrollment(_ record: BuzzPushPendingEnrollmentRecord) throws {
    pending.removeAll {
      $0.gatewayOrigin == record.gatewayOrigin && $0.relayOrigin == record.relayOrigin
    }
    pending.append(record)
  }
  func removePendingEnrollment(
    gatewayOrigin: String,
    relayOrigin: String
  ) throws {
    if pendingRemoveFailuresRemaining > 0 {
      pendingRemoveFailuresRemaining -= 1
      throw NSError(domain: "MemoryGrantStore", code: 3)
    }
    pending.removeAll {
      $0.gatewayOrigin == gatewayOrigin && $0.relayOrigin == relayOrigin
    }
  }
}

private final class RecordingAppAttest: BuzzDevAppAttesting {
  var clientData: [Data] = []
  var preparedAttestations: [BuzzDevAttestation] = []
  var assertionKeyIds: [String] = []
  /// Fails the next attestation after its key was generated.
  var failAttestation = false

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
    if failAttestation {
      failAttestation = false
      throw URLError(.notConnectedToInternet)
    }
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
