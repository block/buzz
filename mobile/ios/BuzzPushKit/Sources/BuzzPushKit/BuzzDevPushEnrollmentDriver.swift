import CryptoKit
import DeviceCheck
import Foundation

#if canImport(Security)
  import Security
#endif

#if canImport(FoundationNetworking)
  import FoundationNetworking
#endif

/// The opaque gateway capability and binding metadata needed by a later lease publisher.
public struct BuzzPushEndpointGrantRecord: Codable, Equatable, Sendable {
  public let relayOrigin: String
  public let relayPubkey: String
  public let installationId: String
  public let endpointGrant: String
  public let endpointHash: String
  public let appProfile: String
  public let endpointEpoch: Int64
  public let generation: Int64
  public let publishedGeneration: Int64?
  public let expiresAt: Int64

  public init(
    relayOrigin: String,
    relayPubkey: String,
    installationId: String,
    endpointGrant: String,
    endpointHash: String,
    appProfile: String,
    endpointEpoch: Int64,
    generation: Int64,
    publishedGeneration: Int64?,
    expiresAt: Int64
  ) {
    precondition(generation > 0, "Endpoint grant generation must be positive")
    precondition(
      publishedGeneration == nil
        || (publishedGeneration! > 0 && publishedGeneration! <= generation),
      "Published push lease generation must be positive and no greater than the grant generation"
    )
    self.relayOrigin = relayOrigin
    self.relayPubkey = relayPubkey
    self.installationId = installationId
    self.endpointGrant = endpointGrant
    self.endpointHash = endpointHash
    self.appProfile = appProfile
    self.endpointEpoch = endpointEpoch
    self.generation = generation
    self.publishedGeneration = publishedGeneration
    self.expiresAt = expiresAt
  }
}

/// Persistence boundary for endpoint grants. The Runner implementation stores
/// records in its Keychain access group and exposes them over the Flutter bridge.
public protocol BuzzPushEndpointGrantStore {
  func records() throws -> [BuzzPushEndpointGrantRecord]
  func save(_ record: BuzzPushEndpointGrantRecord) throws
  func markPublished(relayOrigin: String, appProfile: String, generation: Int64) throws
}

public enum BuzzDevPushEnrollmentError: Error, LocalizedError, Equatable {
  case invalidGatewayURL
  case invalidRelayURL
  case invalidRelayDescriptor
  case invalidResponse(route: String)
  case unexpectedStatus(route: String, expected: Int, actual: Int, body: String)
  case randomGenerationFailed(Int32)
  case appAttestUnsupported
  case invalidAppAttestKeyId
  case generationExhausted

  public var errorDescription: String? {
    switch self {
    case .invalidGatewayURL:
      return "The development push gateway URL must be an HTTP or HTTPS origin."
    case .invalidRelayURL:
      return "The relay URL must be a ws or wss origin."
    case .invalidRelayDescriptor:
      return "NIP-11 must contain exactly one valid current push key."
    case .invalidResponse(let route):
      return "The response from \(route) did not match the closed push protocol."
    case .unexpectedStatus(let route, let expected, let actual, let body):
      return "The response from \(route) was HTTP \(actual), expected \(expected): \(body)"
    case .randomGenerationFailed(let status):
      return "Secure random generation failed with status \(status)."
    case .appAttestUnsupported:
      return "App Attest is unavailable on this device."
    case .invalidAppAttestKeyId:
      return "The App Attest key identifier is missing or invalid."
    case .generationExhausted:
      return "The development push grant generation cannot advance further."
    }
  }
}

protocol BuzzDevAppAttesting {
  func prepareAttestation() async throws -> BuzzDevAttestation
  func attestation(_ prepared: BuzzDevAttestation, clientData: Data) async throws
    -> BuzzDevAttestation
  func assertion(clientData: Data) async throws -> String
}

struct BuzzDevAttestation: Equatable {
  let keyId: String
  let attestation: String
}

private enum BuzzSecureRandom {
  static func bytes(count: Int) throws -> Data {
    var bytes = [UInt8](repeating: 0, count: count)
    let status = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
    guard status == errSecSuccess else {
      throw BuzzDevPushEnrollmentError.randomGenerationFailed(status)
    }
    return Data(bytes)
  }
}

#if DEBUG
  struct BuzzDevAppAttestProvider: BuzzDevAppAttesting {
    private static let attestationPrefix = Data("buzz-dev-app-attest-v1:".utf8)
    private static let assertionBytes = Data("buzz-dev-app-assertion-v1".utf8)

    let randomBytes: () throws -> Data

    init(
      randomBytes: @escaping () throws -> Data = {
        try BuzzSecureRandom.bytes(count: 32)
      }
    ) {
      self.randomBytes = randomBytes
    }

    func prepareAttestation() async throws -> BuzzDevAttestation {
      let entropy = try randomBytes()
      precondition(entropy.count == 32, "Development attestation entropy must be exactly 32 bytes")
      let bytes = Self.attestationPrefix + entropy
      return BuzzDevAttestation(
        keyId: Data(SHA256.hash(data: bytes)).base64EncodedString(),
        attestation: bytes.base64EncodedString()
      )
    }

    func attestation(
      _ prepared: BuzzDevAttestation,
      clientData: Data
    ) async throws -> BuzzDevAttestation {
      precondition(!clientData.isEmpty, "Enrollment client data must not be empty")
      return prepared
    }

    func assertion(clientData: Data) async throws -> String {
      precondition(!clientData.isEmpty, "Delegation client data must not be empty")
      return Self.assertionBytes.base64EncodedString()
    }
  }
#endif

private enum BuzzAppAttestKeyId {
  static func isValid(_ keyId: String) -> Bool {
    guard !keyId.isEmpty,
      keyId.unicodeScalars.allSatisfy(\.isASCII),
      let bytes = Data(base64Encoded: keyId)
    else { return false }
    return bytes.count == 32 && bytes.base64EncodedString() == keyId
  }
}

protocol BuzzAppAttestKeyIdStoring {
  func keyId() throws -> String?
  func saveKeyId(_ keyId: String) throws
}

struct BuzzAppAttestKeyIdKeychainStore: BuzzAppAttestKeyIdStoring {
  private static let service = "buzz.push.app-attest"
  private static let account = "key-id-v1"

  private let accessGroup: String?
  private let copyMatching: (CFDictionary, UnsafeMutablePointer<CFTypeRef?>?) -> OSStatus
  private let update: (CFDictionary, CFDictionary) -> OSStatus
  private let add: (CFDictionary, UnsafeMutablePointer<CFTypeRef?>?) -> OSStatus

  init(
    accessGroup: String?,
    copyMatching: @escaping (CFDictionary, UnsafeMutablePointer<CFTypeRef?>?) -> OSStatus =
      SecItemCopyMatching,
    update: @escaping (CFDictionary, CFDictionary) -> OSStatus = SecItemUpdate,
    add: @escaping (CFDictionary, UnsafeMutablePointer<CFTypeRef?>?) -> OSStatus = SecItemAdd
  ) {
    self.accessGroup = accessGroup
    self.copyMatching = copyMatching
    self.update = update
    self.add = add
  }

  func keyId() throws -> String? {
    var query = baseQuery()
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = copyMatching(query as CFDictionary, &result)
    if status == errSecItemNotFound { return nil }
    guard status == errSecSuccess else {
      throw keychainError(status, operation: "read")
    }
    guard let data = result as? Data,
      let keyId = String(data: data, encoding: .utf8),
      BuzzAppAttestKeyId.isValid(keyId)
    else {
      throw BuzzDevPushEnrollmentError.invalidAppAttestKeyId
    }
    return keyId
  }

  func saveKeyId(_ keyId: String) throws {
    guard BuzzAppAttestKeyId.isValid(keyId) else {
      throw BuzzDevPushEnrollmentError.invalidAppAttestKeyId
    }
    let data = Data(keyId.utf8)
    let updateStatus = update(
      baseQuery() as CFDictionary,
      [kSecValueData as String: data] as CFDictionary
    )
    if updateStatus == errSecSuccess { return }
    guard updateStatus == errSecItemNotFound else {
      throw keychainError(updateStatus, operation: "update")
    }

    var item = baseQuery()
    item[kSecValueData as String] = data
    item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    let addStatus = add(item as CFDictionary, nil)
    guard addStatus == errSecSuccess else {
      throw keychainError(addStatus, operation: "add")
    }
  }

  private func baseQuery() -> [String: Any] {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: Self.service,
      kSecAttrAccount as String: Self.account,
    ]
    if let accessGroup, !accessGroup.isEmpty {
      query[kSecAttrAccessGroup as String] = accessGroup
    }
    return query
  }

  private func keychainError(_ status: OSStatus, operation: String) -> Error {
    NSError(
      domain: NSOSStatusErrorDomain,
      code: Int(status),
      userInfo: [
        NSLocalizedDescriptionKey:
          "App Attest key identifier Keychain \(operation) failed: \(SecCopyErrorMessageString(status, nil) ?? "unknown" as CFString)"
      ]
    )
  }
}

protocol BuzzDCAppAttestServicing {
  var isSupported: Bool { get }
  func generateKey() async throws -> String
  func attestKey(_ keyId: String, clientDataHash: Data) async throws -> Data
  func generateAssertion(_ keyId: String, clientDataHash: Data) async throws -> Data
}

extension DCAppAttestService: BuzzDCAppAttestServicing {}

struct BuzzDCAppAttestProvider: BuzzDevAppAttesting {
  private let service: BuzzDCAppAttestServicing
  private let keyIdStore: BuzzAppAttestKeyIdStoring

  init(
    service: BuzzDCAppAttestServicing = DCAppAttestService.shared,
    keyIdStore: BuzzAppAttestKeyIdStoring
  ) {
    self.service = service
    self.keyIdStore = keyIdStore
  }

  func prepareAttestation() async throws -> BuzzDevAttestation {
    try requireSupportedService()
    let keyId = try await service.generateKey()
    guard BuzzAppAttestKeyId.isValid(keyId) else {
      throw BuzzDevPushEnrollmentError.invalidAppAttestKeyId
    }
    try keyIdStore.saveKeyId(keyId)
    return BuzzDevAttestation(keyId: keyId, attestation: "")
  }

  func attestation(
    _ prepared: BuzzDevAttestation,
    clientData: Data
  ) async throws -> BuzzDevAttestation {
    precondition(!clientData.isEmpty, "Enrollment client data must not be empty")
    try requireSupportedService()
    guard BuzzAppAttestKeyId.isValid(prepared.keyId),
      try keyIdStore.keyId() == prepared.keyId
    else {
      throw BuzzDevPushEnrollmentError.invalidAppAttestKeyId
    }
    let object = try await service.attestKey(
      prepared.keyId,
      clientDataHash: Data(SHA256.hash(data: clientData))
    )
    return BuzzDevAttestation(
      keyId: prepared.keyId,
      attestation: object.base64EncodedString()
    )
  }

  func assertion(clientData: Data) async throws -> String {
    precondition(!clientData.isEmpty, "Delegation client data must not be empty")
    try requireSupportedService()
    guard let keyId = try keyIdStore.keyId(),
      BuzzAppAttestKeyId.isValid(keyId)
    else {
      throw BuzzDevPushEnrollmentError.invalidAppAttestKeyId
    }
    let object = try await service.generateAssertion(
      keyId,
      clientDataHash: Data(SHA256.hash(data: clientData))
    )
    return object.base64EncodedString()
  }

  private func requireSupportedService() throws {
    guard service.isSupported else {
      throw BuzzDevPushEnrollmentError.appAttestUnsupported
    }
  }
}

/// Enrollment and delegation driver for real App Attest and the gated debug bypass.
public final class BuzzDevPushEnrollmentDriver {
  public static let appProfile = "buzz-ios-sandbox"
  public static let endpointEpoch: Int64 = 1

  private let gatewayBaseURL: URL
  private let store: BuzzPushEndpointGrantStore
  private let session: URLSession
  private let appAttest: BuzzDevAppAttesting
  private let now: () -> Date
  private let lifetimeSeconds: Int64
  private let installationIdBytes: () throws -> Data

  /// Creates a driver backed by Apple's App Attest service and persists the
  /// generated App Attest key identifier in the requested Keychain access group.
  public convenience init(
    gatewayBaseURL: URL,
    store: BuzzPushEndpointGrantStore,
    appAttestKeychainAccessGroup: String?,
    session: URLSession = .shared
  ) throws {
    try self.init(
      gatewayBaseURL: gatewayBaseURL,
      store: store,
      session: session,
      appAttest: BuzzDCAppAttestProvider(
        keyIdStore: BuzzAppAttestKeyIdKeychainStore(
          accessGroup: appAttestKeychainAccessGroup
        )
      ),
      now: Date.init,
      lifetimeSeconds: 2_592_000,
      installationIdBytes: { try BuzzSecureRandom.bytes(count: 16) }
    )
  }

  #if DEBUG
    public convenience init(
      gatewayBaseURL: URL,
      store: BuzzPushEndpointGrantStore,
      session: URLSession = .shared
    ) throws {
      try self.init(
        gatewayBaseURL: gatewayBaseURL,
        store: store,
        session: session,
        appAttest: BuzzDevAppAttestProvider(),
        now: Date.init,
        lifetimeSeconds: 2_592_000,
        installationIdBytes: { try BuzzSecureRandom.bytes(count: 16) }
      )
    }
  #endif

  init(
    gatewayBaseURL: URL,
    store: BuzzPushEndpointGrantStore,
    session: URLSession,
    appAttest: BuzzDevAppAttesting,
    now: @escaping () -> Date,
    lifetimeSeconds: Int64,
    installationIdBytes: @escaping () throws -> Data = {
      try BuzzSecureRandom.bytes(count: 16)
    }
  ) throws {
    guard Self.isHTTPOrigin(gatewayBaseURL), lifetimeSeconds > 0 else {
      throw BuzzDevPushEnrollmentError.invalidGatewayURL
    }
    self.gatewayBaseURL = gatewayBaseURL
    self.store = store
    self.session = session
    self.appAttest = appAttest
    self.now = now
    self.lifetimeSeconds = lifetimeSeconds
    self.installationIdBytes = installationIdBytes
  }

  public func endpointGrants() throws -> [BuzzPushEndpointGrantRecord] {
    try store.records()
  }

  /// Fetches the relay's current NIP-11 push key, enrolls the APNs endpoint,
  /// delegates to that key, and durably saves the resulting opaque grant.
  public func enroll(
    deviceToken: Data,
    relayURL: URL
  ) async throws -> BuzzPushEndpointGrantRecord {
    precondition(!deviceToken.isEmpty, "The APNs device token must not be empty")
    let relayOrigin = try Self.relayOrigin(relayURL)
    let relayPubkey = try await fetchCurrentRelayPushPubkey(from: relayOrigin.url)
    let endpoint = Self.lowercaseHex(deviceToken)
    let endpointHash = Self.lowercaseHex(Data(SHA256.hash(data: deviceToken)))
    let nowSeconds = Int64(now().timeIntervalSince1970)

    let storedRecords = try store.records()
    let storedForOrigin = storedRecords.first {
      $0.relayOrigin == relayOrigin.text && $0.appProfile == Self.appProfile
    }
    if let current = storedForOrigin,
      current.relayPubkey == relayPubkey,
      current.endpointHash == endpointHash,
      current.endpointEpoch == Self.endpointEpoch,
      current.expiresAt > nowSeconds + 300
    {
      return current
    }
    let generation: Int64
    if let storedForOrigin {
      let (next, overflow) = storedForOrigin.generation.addingReportingOverflow(1)
      guard !overflow, next > 0 else {
        throw BuzzDevPushEnrollmentError.generationExhausted
      }
      generation = next
    } else {
      generation = 1
    }

    let (expiresAt, expiresOverflow) = nowSeconds.addingReportingOverflow(lifetimeSeconds)
    guard !expiresOverflow else {
      throw BuzzDevPushEnrollmentError.invalidGatewayURL
    }

    let enrollmentChallenge = try await challenge()
    let preparedAttestation = try await appAttest.prepareAttestation()
    let enrollmentClientData = try BuzzPushTranscript.enroll(
      challengeId: enrollmentChallenge.id,
      challenge: enrollmentChallenge.value,
      keyId: preparedAttestation.keyId,
      appProfile: Self.appProfile,
      endpoint: endpoint,
      endpointEpoch: Self.endpointEpoch,
      expiresAt: expiresAt
    )
    let attestation = try await appAttest.attestation(
      preparedAttestation,
      clientData: enrollmentClientData
    )
    guard attestation.keyId == preparedAttestation.keyId else {
      throw BuzzDevPushEnrollmentError.invalidResponse(route: "development attestation")
    }
    let installation = try await enrollInstallation(
      challenge: enrollmentChallenge,
      endpoint: endpoint,
      expiresAt: expiresAt,
      attestation: attestation
    )

    let delegationChallenge = try await challenge()
    let delegationClientData = try BuzzPushTranscript.delegate(
      challengeId: delegationChallenge.id,
      challenge: delegationChallenge.value,
      installationHandle: installation,
      endpointEpoch: Self.endpointEpoch,
      generation: generation,
      relayPubkey: relayPubkey,
      notBefore: nowSeconds,
      expiresAt: expiresAt
    )
    let assertion = try await appAttest.assertion(clientData: delegationClientData)
    let endpointGrant = try await delegate(
      challenge: delegationChallenge,
      installationHandle: installation,
      relayPubkey: relayPubkey,
      generation: generation,
      notBefore: nowSeconds,
      expiresAt: expiresAt,
      assertion: assertion
    )

    let installationId: String
    if let storedForOrigin {
      installationId = storedForOrigin.installationId
    } else {
      let installationBytes = try installationIdBytes()
      precondition(
        installationBytes.count == 16,
        "NIP-PL installation identity entropy must be exactly 16 bytes"
      )
      // NIP-PL:76 also requires a fresh value on reinstall. Keychain survival can
      // retain this value across reinstall. Reinstall detection is intentionally deferred.
      installationId = Self.lowercaseHex(installationBytes)
    }
    let record = BuzzPushEndpointGrantRecord(
      relayOrigin: relayOrigin.text,
      relayPubkey: relayPubkey,
      installationId: installationId,
      endpointGrant: endpointGrant,
      endpointHash: endpointHash,
      appProfile: Self.appProfile,
      endpointEpoch: Self.endpointEpoch,
      generation: generation,
      publishedGeneration: nil,
      expiresAt: expiresAt
    )
    try store.save(record)
    return record
  }

  private func challenge() async throws -> Challenge {
    let response: ChallengeResponse = try await post(
      route: "v1/installations/challenges",
      expectedStatus: 200,
      body: VersionRequest(v: 1)
    )
    guard let id = UUID(uuidString: response.challengeId),
      response.challengeId == id.uuidString.lowercased(),
      Self.isBase64URLChallenge(response.challenge),
      response.expiresAt > Int64(now().timeIntervalSince1970)
    else {
      throw BuzzDevPushEnrollmentError.invalidResponse(route: "v1/installations/challenges")
    }
    return Challenge(id: id, value: response.challenge)
  }

  private func enrollInstallation(
    challenge: Challenge,
    endpoint: String,
    expiresAt: Int64,
    attestation: BuzzDevAttestation
  ) async throws -> UUID {
    let response: InstallationResponse = try await post(
      route: "v1/installations",
      expectedStatus: 201,
      body: InstallationRequest(
        v: 1,
        challengeId: challenge.id.uuidString.lowercased(),
        challenge: challenge.value,
        keyId: attestation.keyId,
        attestation: attestation.attestation,
        appProfile: Self.appProfile,
        endpoint: endpoint,
        endpointEpoch: Self.endpointEpoch,
        expiresAt: expiresAt
      )
    )
    guard let installation = UUID(uuidString: response.installationHandle),
      response.installationHandle == installation.uuidString.lowercased(),
      response.endpointEpoch == Self.endpointEpoch,
      response.expiresAt == expiresAt
    else {
      throw BuzzDevPushEnrollmentError.invalidResponse(route: "v1/installations")
    }
    return installation
  }

  private func delegate(
    challenge: Challenge,
    installationHandle: UUID,
    relayPubkey: String,
    generation: Int64,
    notBefore: Int64,
    expiresAt: Int64,
    assertion: String
  ) async throws -> String {
    let response: DelegationResponse = try await post(
      route: "v1/delegations",
      expectedStatus: 201,
      body: DelegationRequest(
        v: 1,
        challengeId: challenge.id.uuidString.lowercased(),
        challenge: challenge.value,
        installationHandle: installationHandle.uuidString.lowercased(),
        endpointEpoch: Self.endpointEpoch,
        generation: generation,
        relayPubkey: relayPubkey,
        notBefore: notBefore,
        expiresAt: expiresAt,
        assertion: assertion
      )
    )
    guard !response.endpointGrant.isEmpty, response.endpointGrant.utf8.count <= 4_096 else {
      throw BuzzDevPushEnrollmentError.invalidResponse(route: "v1/delegations")
    }
    return response.endpointGrant
  }

  private func fetchCurrentRelayPushPubkey(from relayOrigin: URL) async throws -> String {
    var request = URLRequest(url: relayOrigin)
    request.httpMethod = "GET"
    request.setValue("application/nostr+json", forHTTPHeaderField: "Accept")
    let (data, response) = try await session.data(for: request)
    try Self.expectStatus(response, data: data, route: "NIP-11", expected: 200)
    let document: RelayInformation
    do {
      document = try JSONDecoder().decode(RelayInformation.self, from: data)
    } catch {
      throw BuzzDevPushEnrollmentError.invalidRelayDescriptor
    }
    let current = document.push.keys.filter(\.current)
    guard current.count == 1, Self.isLowercaseHexPubkey(current[0].pubkey) else {
      throw BuzzDevPushEnrollmentError.invalidRelayDescriptor
    }
    return current[0].pubkey
  }

  private func post<Request: Encodable, Response: Decodable>(
    route: String,
    expectedStatus: Int,
    body: Request
  ) async throws -> Response {
    let url = route.split(separator: "/").reduce(gatewayBaseURL) {
      $0.appendingPathComponent(String($1))
    }
    var request = URLRequest(url: url)
    request.httpMethod = "POST"
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    request.httpBody = try JSONEncoder().encode(body)
    let (data, response) = try await session.data(for: request)
    try Self.expectStatus(response, data: data, route: route, expected: expectedStatus)
    do {
      return try JSONDecoder().decode(Response.self, from: data)
    } catch {
      throw BuzzDevPushEnrollmentError.invalidResponse(route: route)
    }
  }

  private static func expectStatus(
    _ response: URLResponse,
    data: Data,
    route: String,
    expected: Int
  ) throws {
    guard let http = response as? HTTPURLResponse else {
      throw BuzzDevPushEnrollmentError.invalidResponse(route: route)
    }
    guard http.statusCode == expected else {
      let body = String(decoding: data.prefix(512), as: UTF8.self)
      throw BuzzDevPushEnrollmentError.unexpectedStatus(
        route: route, expected: expected, actual: http.statusCode, body: body
      )
    }
  }

  private static func isHTTPOrigin(_ url: URL) -> Bool {
    (url.scheme == "http" || url.scheme == "https")
      && url.host != nil
      && (url.path.isEmpty || url.path == "/")
      && url.user == nil
      && url.password == nil
      && url.query == nil
      && url.fragment == nil
  }

  private static func relayOrigin(_ url: URL) throws -> (url: URL, text: String) {
    guard url.scheme == "ws" || url.scheme == "wss",
      url.host != nil,
      url.path.isEmpty || url.path == "/",
      url.user == nil,
      url.password == nil,
      url.query == nil,
      url.fragment == nil
    else {
      throw BuzzDevPushEnrollmentError.invalidRelayURL
    }
    var components = URLComponents()
    components.scheme = url.scheme == "wss" ? "https" : "http"
    components.host = url.host
    components.port = url.port
    components.path = "/"
    guard let httpURL = components.url else {
      throw BuzzDevPushEnrollmentError.invalidRelayURL
    }
    var relayComponents = components
    relayComponents.scheme = url.scheme
    relayComponents.path = ""
    guard let relayText = relayComponents.string else {
      throw BuzzDevPushEnrollmentError.invalidRelayURL
    }
    return (httpURL, relayText)
  }

  private static func isLowercaseHexPubkey(_ value: String) -> Bool {
    value.utf8.count == 64
      && value.utf8.allSatisfy {
        (48...57).contains($0) || (97...102).contains($0)
      }
  }

  private static func isBase64URLChallenge(_ value: String) -> Bool {
    guard value.utf8.count == 43,
      value.utf8.allSatisfy({
        (48...57).contains($0) || (65...90).contains($0)
          || (97...122).contains($0) || $0 == 45 || $0 == 95
      })
    else { return false }
    var padded = value.replacingOccurrences(of: "-", with: "+")
      .replacingOccurrences(of: "_", with: "/")
    padded += String(repeating: "=", count: (4 - padded.count % 4) % 4)
    return Data(base64Encoded: padded)?.count == 32
  }

  private static func lowercaseHex(_ data: Data) -> String {
    data.map { String(format: "%02x", $0) }.joined()
  }
}

private struct VersionRequest: Encodable { let v: Int }
private struct Challenge {
  let id: UUID
  let value: String
}
private struct ChallengeResponse: Decodable {
  let challengeId: String
  let challenge: String
  let expiresAt: Int64
  enum CodingKeys: String, CodingKey {
    case challengeId = "challenge_id"
    case challenge
    case expiresAt = "expires_at"
  }
}
private struct InstallationRequest: Encodable {
  let v: Int
  let challengeId: String
  let challenge: String
  let keyId: String
  let attestation: String
  let appProfile: String
  let endpoint: String
  let endpointEpoch: Int64
  let expiresAt: Int64
  enum CodingKeys: String, CodingKey {
    case v
    case challengeId = "challenge_id"
    case challenge
    case keyId = "key_id"
    case attestation
    case appProfile = "app_profile"
    case endpoint
    case endpointEpoch = "endpoint_epoch"
    case expiresAt = "expires_at"
  }
}
private struct InstallationResponse: Decodable {
  let installationHandle: String
  let endpointEpoch: Int64
  let expiresAt: Int64
  enum CodingKeys: String, CodingKey {
    case installationHandle = "installation_handle"
    case endpointEpoch = "endpoint_epoch"
    case expiresAt = "expires_at"
  }
}
private struct DelegationRequest: Encodable {
  let v: Int
  let challengeId: String
  let challenge: String
  let installationHandle: String
  let endpointEpoch: Int64
  let generation: Int64
  let relayPubkey: String
  let notBefore: Int64
  let expiresAt: Int64
  let assertion: String
  enum CodingKeys: String, CodingKey {
    case v
    case challengeId = "challenge_id"
    case challenge
    case installationHandle = "installation_handle"
    case endpointEpoch = "endpoint_epoch"
    case generation
    case relayPubkey = "relay_pubkey"
    case notBefore = "not_before"
    case expiresAt = "expires_at"
    case assertion
  }
}
private struct DelegationResponse: Decodable {
  let endpointGrant: String
  enum CodingKeys: String, CodingKey { case endpointGrant = "endpoint_grant" }
}
private struct RelayInformation: Decodable {
  struct Push: Decodable {
    struct Key: Decodable {
      let pubkey: String
      let current: Bool
    }
    let keys: [Key]
  }
  let push: Push
}
