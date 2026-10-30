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
  /// Gateway authority that issued this opaque capability.
  public let gatewayOrigin: String
  /// Canonical WebSocket relay origin that owns the client lease.
  public let relayOrigin: String
  /// NIP-PL delegation key selected from the relay push descriptor.
  public let relayPubkey: String
  /// Optional NIP-11 `self` key that verifies relay-authored NIP-29 metadata.
  public let relayMetadataPubkey: String?
  /// Gateway installation authority. This is distinct from [installationId],
  /// which is the unlinkable per-relay-origin NIP-PL lease address.
  public let gatewayInstallationHandle: String?
  /// App Attest key that authenticates mutations for the gateway installation.
  public let appAttestKeyId: String
  /// Unlinkable per-relay lease address, distinct from the gateway handle.
  public let installationId: String
  /// Opaque capability issued by the gateway for relay delivery.
  public let endpointGrant: String
  /// Lowercase SHA-256 digest of the binary APNs device token.
  public let endpointHash: String
  /// Monotonic endpoint version bound into the gateway grant.
  public let endpointEpoch: Int64
  /// Monotonic delegation version bound into the gateway grant.
  public let generation: Int64
  /// Unix expiration time of the installation or delegation authority.
  public let expiresAt: Int64

  /// Creates a gateway-scoped durable record from authenticated enrollment state.
  public init(
    gatewayOrigin: String,
    relayOrigin: String,
    relayPubkey: String,
    relayMetadataPubkey: String? = nil,
    gatewayInstallationHandle: String? = nil,
    appAttestKeyId: String,
    installationId: String,
    endpointGrant: String,
    endpointHash: String,
    endpointEpoch: Int64,
    generation: Int64,
    expiresAt: Int64
  ) {
    precondition(generation > 0, "Endpoint grant generation must be positive")
    self.gatewayOrigin = gatewayOrigin
    self.relayOrigin = relayOrigin
    self.relayPubkey = relayPubkey
    self.relayMetadataPubkey = relayMetadataPubkey
    self.gatewayInstallationHandle = gatewayInstallationHandle
    self.appAttestKeyId = appAttestKeyId
    self.installationId = installationId
    self.endpointGrant = endpointGrant
    self.endpointHash = endpointHash
    self.endpointEpoch = endpointEpoch
    self.generation = generation
    self.expiresAt = expiresAt
  }
}

/// Persistence boundary for endpoint grants. The Runner implementation stores
/// records in its Keychain access group and exposes them over the Flutter bridge.
public protocol BuzzPushEndpointGrantStore {
  /// Returns durable grants across configured origins without migrating them.
  func records() throws -> [BuzzPushEndpointGrantRecord]
  /// Atomically replaces the grant for its gateway and relay origin.
  func save(_ record: BuzzPushEndpointGrantRecord) throws
  /// Atomically updates all grants affected by one installation mutation.
  func replaceAll(_ records: [BuzzPushEndpointGrantRecord]) throws
  /// Returns pending operations so cancellation can settle every issuance.
  func pendingEnrollments() throws -> [BuzzPushPendingEnrollmentRecord]
  /// Loads the exact retry journal for a gateway and relay origin.
  func pendingEnrollment(
    gatewayOrigin: String,
    relayOrigin: String
  ) throws -> BuzzPushPendingEnrollmentRecord?
  /// Persists the retry journal before any enrollment or delegation mutation.
  func savePendingEnrollment(_ record: BuzzPushPendingEnrollmentRecord) throws
  /// Removes a journal only after success or authoritative terminal failure.
  func removePendingEnrollment(
    gatewayOrigin: String,
    relayOrigin: String
  ) throws
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
  func assertion(keyId: String, clientData: Data) async throws -> String
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

  func assertion(keyId: String, clientData: Data) async throws -> String {
    precondition(!clientData.isEmpty, "Delegation client data must not be empty")
    try requireSupportedService()
    guard BuzzAppAttestKeyId.isValid(keyId) else {
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
  public static let endpointEpoch: Int64 = 1

  // Leave clock-skew margin within the gateway's 300-second freshness cap.
  private static let managementRequestLifetimeSeconds: Int64 = 240
  /// Minimum age before a journaled installation request the gateway rejected
  /// is replaced. It doubles with each consecutive replacement, which bounds
  /// fresh App Attest keys while rejections persist.
  static let rejectedEnrollmentReplacementSeconds: Int64 = 3_600

  private let gatewayBaseURL: URL
  private let gatewayOrigin: String
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
    guard lifetimeSeconds > 0 else {
      throw BuzzDevPushEnrollmentError.invalidGatewayURL
    }
    let canonical: (url: URL, text: String)
    do {
      canonical = try BuzzPushTranscript.canonicalGatewayOrigin(gatewayBaseURL)
    } catch {
      throw BuzzDevPushEnrollmentError.invalidGatewayURL
    }
    self.gatewayBaseURL = canonical.url
    self.gatewayOrigin = canonical.text
    self.store = store
    self.session = session
    self.appAttest = appAttest
    self.now = now
    self.lifetimeSeconds = lifetimeSeconds
    self.installationIdBytes = installationIdBytes
  }

  /// Returns the persisted opaque grants available to the lease publisher.
  public func endpointGrants() throws -> [BuzzPushEndpointGrantRecord] {
    try store.records().filter { $0.gatewayOrigin == gatewayOrigin }
  }

  /// Returns durable pending requests, including enrollment whose response was lost.
  public func pendingEnrollments() throws -> [BuzzPushPendingEnrollmentRecord] {
    try store.pendingEnrollments().filter { $0.gatewayOrigin == gatewayOrigin }
  }

  /// Settles an interrupted enrollment before revoking its resulting authority.
  public func revokePendingDelegation(relayOrigin: String, relayPubkey: String) async throws {
    try await settlePendingEnrollments(relayOrigin: relayOrigin, relayPubkey: relayPubkey)
    let records = try store.records()
    for record in records where record.gatewayOrigin == gatewayOrigin
      && record.relayOrigin == relayOrigin && (relayPubkey.isEmpty || record.relayPubkey == relayPubkey)
    {
      if records.contains(where: {
        $0.gatewayOrigin == gatewayOrigin && $0.relayOrigin != relayOrigin
          && $0.gatewayInstallationHandle == record.gatewayInstallationHandle
          && $0.relayPubkey == record.relayPubkey && !$0.endpointGrant.isEmpty
          && $0.expiresAt > Int64(now().timeIntervalSince1970)
      }) { continue }
      if let handle = record.gatewayInstallationHandle, let installation = UUID(uuidString: handle) {
        try await revokeDelegation(installationHandle: installation, keyId: record.appAttestKeyId,
          relayPubkey: record.relayPubkey, generation: record.generation)
      }
    }
  }

  private func settlePendingEnrollments(relayOrigin: String? = nil, relayPubkey: String) async throws {
    for pending in try pendingEnrollments() where (relayOrigin == nil || pending.relayOrigin == relayOrigin)
      && (relayPubkey.isEmpty || pending.relayPubkey == relayPubkey)
    {
      let superseded = try store.records().contains {
        $0.gatewayOrigin == gatewayOrigin && $0.gatewayInstallationHandle == pending.gatewayInstallationHandle
          && $0.endpointEpoch > (pending.endpointEpoch)
      }
      if pending.expiresAt <= Int64(now().timeIntervalSince1970) || superseded {
        try await retireReplacedDelegation(relayOrigin: pending.relayOrigin,
          installationHandle: pending.gatewayInstallationHandle, relayPubkey: pending.relayPubkey)
        try store.removePendingEnrollment(gatewayOrigin: gatewayOrigin, relayOrigin: pending.relayOrigin)
      } else {
        do {
          _ = try await completeEnrollment(pending, storedRecords: store.records(), relayMetadataPubkey: nil)
        } catch let error as BuzzDevPushEnrollmentError {
          // A 404 rules out the journaled installation, not prior authority
          // still retained for this origin.
          guard case .unexpectedStatus(route: "v1/installations", _, 404, _) = error else { throw error }
          try await retireReplacedDelegation(relayOrigin: pending.relayOrigin,
            installationHandle: pending.gatewayInstallationHandle, relayPubkey: pending.relayPubkey)
          try store.removePendingEnrollment(gatewayOrigin: gatewayOrigin, relayOrigin: pending.relayOrigin)
        }
      }
    }
  }

  /// Revokes one exact delegation without contacting its relay. The caller keeps
  /// its cleanup journal until this request and the local save both succeed.
  public func revokeDelegation(
    installationHandle: UUID, keyId: String, relayPubkey: String, generation: Int64,
    settlePending: Bool = false
  ) async throws {
    if settlePending {
      let handle = installationHandle.uuidString.lowercased()
      let origins = Set(try store.records().filter {
        $0.gatewayOrigin == gatewayOrigin && $0.gatewayInstallationHandle == handle
          && $0.relayPubkey == relayPubkey
      }.map(\.relayOrigin)).union(try pendingEnrollments().filter {
        $0.gatewayInstallationHandle == handle && $0.relayPubkey == relayPubkey
      }.map(\.relayOrigin))
      for origin in origins {
        try await settlePendingEnrollments(relayOrigin: origin, relayPubkey: relayPubkey)
      }
      var includedOriginal = false
      let records = try store.records()
      for record in records where record.gatewayOrigin == gatewayOrigin
        && record.relayPubkey == relayPubkey
      {
        if record.gatewayInstallationHandle != handle {
          guard origins.contains(record.relayOrigin), !records.contains(where: {
            $0.gatewayOrigin == gatewayOrigin && !origins.contains($0.relayOrigin)
              && $0.gatewayInstallationHandle == record.gatewayInstallationHandle
              && $0.relayPubkey == relayPubkey && !$0.endpointGrant.isEmpty
              && $0.expiresAt > Int64(now().timeIntervalSince1970)
          }) else { continue }
        }
        if let handle = record.gatewayInstallationHandle, let installation = UUID(uuidString: handle) {
          if installation == installationHandle { includedOriginal = true }
          try await revokeDelegation(installationHandle: installation, keyId: record.appAttestKeyId,
            relayPubkey: relayPubkey, generation: record.generation)
        }
      }
      if !includedOriginal {
        try await revokeDelegation(installationHandle: installationHandle, keyId: keyId,
          relayPubkey: relayPubkey, generation: generation)
      }
      return
    }
    let matching = try store.records().filter {
      $0.gatewayOrigin == gatewayOrigin
        && $0.gatewayInstallationHandle == installationHandle.uuidString.lowercased()
        && $0.relayPubkey == relayPubkey
    }
    if !matching.isEmpty && matching.allSatisfy({ $0.expiresAt <= Int64(now().timeIntervalSince1970) }) { return }
    if try store.records().contains(where: {
      $0.gatewayOrigin == gatewayOrigin
        && $0.gatewayInstallationHandle == installationHandle.uuidString.lowercased()
        && $0.relayPubkey == relayPubkey && $0.generation > generation
    }) { return }
    try store.replaceAll(store.records().map { record in
      guard record.gatewayOrigin == gatewayOrigin
        && record.gatewayInstallationHandle == installationHandle.uuidString.lowercased()
        && record.relayPubkey == relayPubkey && record.generation == generation
      else { return record }
      // Keep the version/lifetime but stop local reuse before awaiting network.
      return record.with(endpointGrant: "")
    })
    let requestExpiresAt = Int64(now().timeIntervalSince1970) + Self.managementRequestLifetimeSeconds
    let clientData = try BuzzPushTranscript.revokeDelegation(
      gatewayOrigin: gatewayBaseURL, requestExpiresAt: requestExpiresAt,
      installationHandle: installationHandle, relayPubkey: relayPubkey, generation: generation
    )
    let assertion = try await appAttest.assertion(keyId: keyId, clientData: clientData)
    let response: MutationResponse = try await post(
      route: "v1/delegations/revoke", expectedStatus: 200,
      body: RevokeDelegationRequest(
        v: 2, gatewayOrigin: gatewayOrigin, requestExpiresAt: requestExpiresAt,
        installationHandle: installationHandle.uuidString.lowercased(),
        relayPubkey: relayPubkey, generation: generation, assertion: assertion
      )
    )
    guard response.status == "revoked" else {
      throw BuzzDevPushEnrollmentError.invalidResponse(route: "v1/delegations/revoke")
    }
  }

  /// Fetches the relay's current NIP-11 push key, enrolls the APNs endpoint,
  /// delegates to that key, and durably saves the resulting opaque grant.
  public func enroll(
    deviceToken: Data,
    relayURL: URL
  ) async throws -> BuzzPushEndpointGrantRecord {
    try await enroll(deviceToken: deviceToken, relayURL: relayURL, replacing: nil)
  }

  /// `rejected` names a journal the gateway terminally rejected. It stays
  /// durable, with its replacement attempt recorded, until a fresh request
  /// overwrites it.
  private func enroll(
    deviceToken: Data,
    relayURL: URL,
    replacing rejected: Replacement?
  ) async throws -> BuzzPushEndpointGrantRecord {
    precondition(!deviceToken.isEmpty, "The APNs device token must not be empty")
    let relayOrigin = try Self.relayOrigin(relayURL)
    let relayKeys = try await fetchCurrentRelayKeys(from: relayOrigin.url)
    let relayPubkey = relayKeys.pushPubkey
    let endpoint = Self.lowercaseHex(deviceToken)
    let endpointHash = Self.lowercaseHex(Data(SHA256.hash(data: deviceToken)))
    let nowSeconds = Int64(now().timeIntervalSince1970)

    let storedRecords = try store.records()
    let storedForOrigin = storedRecords.first {
      $0.gatewayOrigin == gatewayOrigin && $0.relayOrigin == relayOrigin.text
    }
    let journal = try store.pendingEnrollment(
      gatewayOrigin: gatewayOrigin,
      relayOrigin: relayOrigin.text
    )
    // Match the rejected request by its challenge, so a stored copy that differs
    // in any other field cannot be resumed again.
    let pendingEnrollment =
      rejected != nil && journal?.challengeId == rejected?.journal.challengeId ? nil : journal
    if let pending = pendingEnrollment, let handle = pending.gatewayInstallationHandle,
      storedRecords.contains(where: {
        $0.gatewayOrigin == gatewayOrigin && $0.gatewayInstallationHandle == handle
          && $0.endpointEpoch > (pending.endpointEpoch)
      })
    {
      // A sibling already advanced this installation. Every request in the old
      // journal is fenced by its endpoint epoch and cannot restore authority.
      try await retireReplacedDelegation(relayOrigin: pending.relayOrigin,
        installationHandle: pending.gatewayInstallationHandle, relayPubkey: pending.relayPubkey)
      try store.removePendingEnrollment(gatewayOrigin: gatewayOrigin, relayOrigin: relayOrigin.text)
      return try await enroll(deviceToken: deviceToken, relayURL: relayURL, replacing: nil)
    }
    if let pending = pendingEnrollment,
      pending.relayPubkey != relayPubkey || pending.endpointHash != endpointHash
        || pending.expiresAt <= nowSeconds
    {
      // Finish an unexpired request using its exact journaled endpoint before
      // replacing it. A token/key change does not make a response-lost request
      // safe to discard, nor does it require revoking sibling installations.
      if pending.expiresAt > nowSeconds {
        if case .rejected(let replacement) = try await resumeEnrollment(
          pending,
          storedRecords: storedRecords,
          relayMetadataPubkey: nil
        ) {
          return try await enroll(
            deviceToken: deviceToken,
            relayURL: relayURL,
            replacing: replacement
          )
        }
      } else {
        try await retireReplacedDelegation(relayOrigin: pending.relayOrigin,
          installationHandle: pending.gatewayInstallationHandle, relayPubkey: pending.relayPubkey)
        try store.removePendingEnrollment(
          gatewayOrigin: gatewayOrigin,
          relayOrigin: relayOrigin.text
        )
      }
      return try await enroll(deviceToken: deviceToken, relayURL: relayURL, replacing: nil)
    }
    let newestSharedGrant = storedRecords.filter {
      $0.gatewayOrigin == gatewayOrigin && $0.relayPubkey == relayPubkey
        && $0.endpointHash == endpointHash
        && $0.expiresAt > nowSeconds + 300 && !$0.endpointGrant.isEmpty
    }.max { $0.generation < $1.generation }
    if pendingEnrollment == nil, let reusableGrant = newestSharedGrant {
      // Reuse the newest delegation without invalidating sibling communities.
      let record = BuzzPushEndpointGrantRecord(
        gatewayOrigin: gatewayOrigin,
        relayOrigin: relayOrigin.text,
        relayPubkey: relayPubkey,
        relayMetadataPubkey: relayKeys.metadataPubkey,
        gatewayInstallationHandle: reusableGrant.gatewayInstallationHandle,
        appAttestKeyId: reusableGrant.appAttestKeyId,
        installationId: try storedForOrigin?.installationId ?? makeInstallationId(),
        endpointGrant: reusableGrant.endpointGrant,
        endpointHash: endpointHash,
        endpointEpoch: reusableGrant.endpointEpoch,
        generation: reusableGrant.generation,
        expiresAt: reusableGrant.expiresAt
      )
      try await retireReplacedDelegation(relayOrigin: record.relayOrigin,
        installationHandle: record.gatewayInstallationHandle, relayPubkey: record.relayPubkey)
      try store.save(record)
      try store.removePendingEnrollment(
        gatewayOrigin: gatewayOrigin,
        relayOrigin: relayOrigin.text
      )
      return record
    }

    // A previously attested installation can delegate independently to a new
    // relay key, or issue a higher-generation grant for the same relay,
    // without attempting duplicate APNs-token enrollment. An installation in
    // its final five minutes is renewed by the authenticated delegation.
    let reusableInstallation = storedRecords.filter { record in
      guard record.gatewayOrigin == gatewayOrigin,
        record.expiresAt > nowSeconds,
        let handle = record.gatewayInstallationHandle,
        let uuid = UUID(uuidString: handle)
      else { return false }
      return handle == uuid.uuidString.lowercased()
    }.max {
      // Prefer the current token owner before considering endpoint rotation.
      let currentEndpoint = $0.endpointHash == endpointHash
      if currentEndpoint != ($1.endpointHash == endpointHash) { return !currentEndpoint }
      return $0.endpointEpoch < $1.endpointEpoch
    }

    let (renewedExpiration, expiresOverflow) = nowSeconds.addingReportingOverflow(lifetimeSeconds)
    guard !expiresOverflow else {
      throw BuzzDevPushEnrollmentError.invalidGatewayURL
    }

    var pending: BuzzPushPendingEnrollmentRecord
    if let existingPending = pendingEnrollment {
      pending = existingPending
    } else if let reusableInstallation,
      let handle = reusableInstallation.gatewayInstallationHandle,
      let existing = UUID(uuidString: handle)
    {
      let expiresAt =
        reusableInstallation.expiresAt > nowSeconds + 300
        ? reusableInstallation.expiresAt
        : renewedExpiration
      let rotating = reusableInstallation.endpointHash != endpointHash
      let (nextEpoch, overflow) = reusableInstallation.endpointEpoch.addingReportingOverflow(rotating ? 1 : 0)
      guard !overflow, nextEpoch > 0 else { throw BuzzDevPushEnrollmentError.generationExhausted }
      pending = BuzzPushPendingEnrollmentRecord(
        gatewayOrigin: gatewayOrigin,
        relayOrigin: relayOrigin.text,
        relayPubkey: relayPubkey,
        endpoint: endpoint,
        endpointHash: endpointHash,
        endpointEpoch: nextEpoch,
        rotationFromEpoch: rotating ? reusableInstallation.endpointEpoch : nil,
        expiresAt: expiresAt,
        installationId: try storedForOrigin?.installationId ?? makeInstallationId(),
        gatewayInstallationHandle: existing.uuidString.lowercased(),
        keyId: reusableInstallation.appAttestKeyId
      )
      try store.savePendingEnrollment(pending)
    } else {
      let expiresAt = renewedExpiration
      let enrollmentChallenge = try await challenge()
      let preparedAttestation = try await appAttest.prepareAttestation()
      let enrollmentClientData = try BuzzPushTranscript.enroll(
        gatewayOrigin: gatewayBaseURL,
        challengeId: enrollmentChallenge.id,
        challenge: enrollmentChallenge.value,
        keyId: preparedAttestation.keyId,
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
      pending = BuzzPushPendingEnrollmentRecord(
        gatewayOrigin: gatewayOrigin,
        relayOrigin: relayOrigin.text,
        relayPubkey: relayPubkey,
        endpoint: endpoint,
        endpointHash: endpointHash,
        expiresAt: expiresAt,
        installationId: try storedForOrigin?.installationId ?? makeInstallationId(),
        challengeId: enrollmentChallenge.id.uuidString.lowercased(),
        challenge: enrollmentChallenge.value,
        keyId: attestation.keyId,
        attestation: attestation.attestation,
        preparedAt: nowSeconds,
        rejectedReplacements: rejected?.rejectedReplacements
      )
      // The exact signed request is durable before the first network attempt.
      // Saving it is also what replaces a rejected journal.
      try store.savePendingEnrollment(pending)
    }

    if pendingEnrollment != nil {
      switch try await resumeEnrollment(
        pending,
        storedRecords: storedRecords,
        relayMetadataPubkey: relayKeys.metadataPubkey
      ) {
      case .completed(let record):
        return record
      case .rejected(let replacement):
        return try await enroll(
          deviceToken: deviceToken,
          relayURL: relayURL,
          replacing: replacement
        )
      }
    }
    return try await completeEnrollment(
      pending,
      storedRecords: storedRecords,
      relayMetadataPubkey: relayKeys.metadataPubkey
    )
  }

  private enum Resumption {
    case completed(BuzzPushEndpointGrantRecord)
    /// The gateway terminally rejected the journal, which may now be replaced.
    case rejected(Replacement)
  }

  private struct Replacement {
    /// The rejected journal as saved, with the replacement attempt recorded.
    let journal: BuzzPushPendingEnrollmentRecord
    /// The rejection count the fresh request carries.
    let rejectedReplacements: Int
  }

  private func resumeEnrollment(
    _ pending: BuzzPushPendingEnrollmentRecord,
    storedRecords: [BuzzPushEndpointGrantRecord],
    relayMetadataPubkey: String?
  ) async throws -> Resumption {
    do {
      return .completed(
        try await completeEnrollment(
          pending,
          storedRecords: storedRecords,
          relayMetadataPubkey: relayMetadataPubkey
        )
      )
    } catch let error as BuzzDevPushEnrollmentError {
      // Only a terminal response permits replacing the old journal with a fresh
      // challenge and key. A fresh request's failure propagates instead of
      // recursively retrying forever.
      guard case .unexpectedStatus(route: "v1/installations", _, let actual, _) = error,
        pending.gatewayInstallationHandle == nil,
        let replacement = try replacement(of: pending, status: actual)
      else { throw error }
      return .rejected(replacement)
    }
  }

  /// 404 means the exact replay found no committed installation and its
  /// challenge is gone, which an ordinary lost request also causes, so a
  /// journal with no recorded rejection is replaced at once. 401 means the
  /// gateway rejected the attestation before looking up a committed
  /// installation, so the same bytes can never recover one. Every replacement
  /// generates and attests a new App Attest key, so a 401, or a 404 for a
  /// journal with a recorded rejection (such as a 409 against a live
  /// installation for the same token, whose consumed challenge then replays as
  /// 404), replaces only once the journal is old enough, with the wait
  /// doubling for each rejection. Each attempt is saved before any key is
  /// generated, so a replacement that fails also waits. A 400 can be a
  /// temporary policy rejection of a request that did commit, so it keeps the
  /// journal.
  private func replacement(
    of pending: BuzzPushPendingEnrollmentRecord,
    status: Int
  ) throws -> Replacement? {
    let replacements = pending.rejectedReplacements ?? 0
    let isRejection = status == 401 || (status == 404 && replacements > 0)
    guard isRejection || status == 404 else { return nil }
    let nowSeconds = Int64(now().timeIntervalSince1970)
    if isRejection, let preparedAt = pending.preparedAt {
      let wait = Self.rejectedEnrollmentReplacementSeconds << min(replacements, 20)
      guard nowSeconds - preparedAt >= wait else { return nil }
    }
    let attempt = pending.with(preparedAt: nowSeconds, rejectedReplacements: replacements + 1)
    try store.savePendingEnrollment(attempt)
    return Replacement(journal: attempt, rejectedReplacements: isRejection ? replacements + 1 : 0)
  }

  private func completeEnrollment(
    _ journal: BuzzPushPendingEnrollmentRecord,
    storedRecords: [BuzzPushEndpointGrantRecord],
    relayMetadataPubkey: String?,
    retryStaleGeneration: Bool = true
  ) async throws -> BuzzPushEndpointGrantRecord {
    var pending = journal
    let endpoint = pending.endpoint
    guard Self.endpointHash(endpoint) == pending.endpointHash
    else {
      throw BuzzDevPushEnrollmentError.invalidResponse(route: "pending enrollment endpoint")
    }
    let nowSeconds = Int64(now().timeIntervalSince1970)
    let relayPubkey = pending.relayPubkey
    let installation: UUID
    if let handle = pending.gatewayInstallationHandle,
      let existing = UUID(uuidString: handle),
      handle == existing.uuidString.lowercased()
    {
      installation = existing
    } else {
      guard let challengeId = pending.challengeId,
        let challengeUUID = UUID(uuidString: challengeId),
        challengeId == challengeUUID.uuidString.lowercased(),
        let challengeValue = pending.challenge,
        let keyId = pending.keyId,
        let attestation = pending.attestation
      else {
        throw BuzzDevPushEnrollmentError.invalidResponse(
          route: "pending development enrollment"
        )
      }
      do {
        installation = try await enrollInstallation(
          challenge: Challenge(id: challengeUUID, value: challengeValue),
          endpoint: endpoint,
          expiresAt: pending.expiresAt,
          attestation: BuzzDevAttestation(keyId: keyId, attestation: attestation)
        )
      } catch let error as BuzzDevPushEnrollmentError {
        // A request answered 409 or 404 consumed or outlived its challenge, so
        // its replay returns 404. Record the rejection so that 404 waits instead
        // of replacing at once. A replay's own 404 is decided on the journal as
        // loaded, so this mark does not throttle an ordinary lost request.
        if case .unexpectedStatus(route: "v1/installations", _, let actual, _) = error,
          actual == 404 || actual == 409,
          (pending.rejectedReplacements ?? 0) == 0
        {
          try store.savePendingEnrollment(
            pending.with(preparedAt: pending.preparedAt, rejectedReplacements: 1)
          )
        }
        throw error
      }
      pending = BuzzPushPendingEnrollmentRecord(
        gatewayOrigin: gatewayOrigin,
        relayOrigin: pending.relayOrigin,
        relayPubkey: pending.relayPubkey,
        endpoint: pending.endpoint,
        endpointHash: pending.endpointHash,
        endpointEpoch: pending.endpointEpoch,
        rotationFromEpoch: pending.rotationFromEpoch,
        expiresAt: pending.expiresAt,
        installationId: pending.installationId,
        gatewayInstallationHandle: installation.uuidString.lowercased(),
        challengeId: pending.challengeId,
        challenge: pending.challenge,
        keyId: pending.keyId,
        attestation: pending.attestation,
        delegationGeneration: pending.delegationGeneration,
        expectedDelegationGeneration: pending.expectedDelegationGeneration,
        preparedAt: pending.preparedAt,
        rejectedReplacements: pending.rejectedReplacements
      )
      try store.savePendingEnrollment(pending)
    }

    let endpointEpoch = pending.endpointEpoch
    if let oldEpoch = pending.rotationFromEpoch {
      guard let keyId = pending.keyId else { throw BuzzDevPushEnrollmentError.invalidAppAttestKeyId }
      let requestExpiresAt = Int64(now().timeIntervalSince1970) + Self.managementRequestLifetimeSeconds
      let data = try BuzzPushTranscript.rotateEndpoint(
        gatewayOrigin: gatewayBaseURL, requestExpiresAt: requestExpiresAt,
        installationHandle: installation, endpointEpoch: oldEpoch,
        newEndpointEpoch: endpointEpoch, endpoint: endpoint
      )
      let assertion = try await appAttest.assertion(keyId: keyId, clientData: data)
      let response: MutationResponse = try await post(
        route: "v1/installations/endpoint", expectedStatus: 200,
        body: RotateEndpointRequest(
          v: 2, gatewayOrigin: gatewayOrigin, requestExpiresAt: requestExpiresAt,
          installationHandle: installation.uuidString.lowercased(), endpointEpoch: oldEpoch,
          newEndpointEpoch: endpointEpoch, endpoint: endpoint, assertion: assertion
        )
      )
      guard response.status == "rotated" else {
        throw BuzzDevPushEnrollmentError.invalidResponse(route: "v1/installations/endpoint")
      }
      try store.replaceAll(store.records().map { record in
        guard record.gatewayOrigin == gatewayOrigin,
          record.gatewayInstallationHandle == installation.uuidString.lowercased(),
          record.endpointEpoch <= oldEpoch else { return record }
        return record.with(endpointGrant: "", endpointHash: pending.endpointHash, endpointEpoch: endpointEpoch)
      })
    }

    let installationHandle = installation.uuidString.lowercased()
    let currentGeneration =
      storedRecords
      .filter {
        $0.gatewayOrigin == gatewayOrigin && $0.gatewayInstallationHandle == installationHandle
          && $0.relayPubkey == relayPubkey
      }
      .map(\.generation)
      .max()
    let expectedGeneration = pending.expectedDelegationGeneration ?? currentGeneration ?? 0
    let generationBase = max(expectedGeneration, max(currentGeneration ?? 0, pending.delegationGeneration))
    let generation: Int64
    if generationBase > 0 {
      let (next, overflow) = generationBase.addingReportingOverflow(1)
      guard !overflow, next > 0 else {
        throw BuzzDevPushEnrollmentError.generationExhausted
      }
      generation = next
    } else {
      generation = 1
    }
    pending = BuzzPushPendingEnrollmentRecord(
      gatewayOrigin: gatewayOrigin,
      relayOrigin: pending.relayOrigin,
      relayPubkey: pending.relayPubkey,
      endpoint: pending.endpoint,
      endpointHash: pending.endpointHash,
      endpointEpoch: pending.endpointEpoch,
      rotationFromEpoch: pending.rotationFromEpoch,
      expiresAt: pending.expiresAt,
      installationId: pending.installationId,
      gatewayInstallationHandle: installationHandle,
      challengeId: pending.challengeId,
      challenge: pending.challenge,
      keyId: pending.keyId,
      attestation: pending.attestation,
      delegationGeneration: generation,
      expectedDelegationGeneration: expectedGeneration,
      preparedAt: pending.preparedAt,
      rejectedReplacements: pending.rejectedReplacements
    )
    // Reserve before delegation so a committed delegation followed by a local
    // save failure is retried at a strictly higher generation.
    try store.savePendingEnrollment(pending)

    let requestExpiresAt = nowSeconds + Self.managementRequestLifetimeSeconds
    let delegationClientData = try BuzzPushTranscript.delegate(
      gatewayOrigin: gatewayBaseURL,
      requestExpiresAt: requestExpiresAt,
      installationHandle: installation,
      endpointEpoch: endpointEpoch,
      expectedGeneration: expectedGeneration,
      generation: generation,
      relayPubkey: relayPubkey,
      notBefore: nowSeconds,
      expiresAt: pending.expiresAt
    )
    guard let appAttestKeyId = pending.keyId else {
      throw BuzzDevPushEnrollmentError.invalidAppAttestKeyId
    }
    let assertion = try await appAttest.assertion(
      keyId: appAttestKeyId,
      clientData: delegationClientData
    )
    let endpointGrant: String
    do {
      endpointGrant = try await delegate(
        requestExpiresAt: requestExpiresAt,
        installationHandle: installation,
        endpointEpoch: endpointEpoch,
        relayPubkey: relayPubkey,
        expectedGeneration: expectedGeneration,
        generation: generation,
        notBefore: nowSeconds,
        expiresAt: pending.expiresAt,
        assertion: assertion
      )
    } catch let error as BuzzDevPushEnrollmentError {
      if case .unexpectedStatus(route: "v1/delegations", _, 409, let body) = error,
        let conflict = try? JSONDecoder().decode(DelegationConflict.self, from: Data(body.utf8)),
        conflict.error == "stale_generation", conflict.currentGeneration >= 0
      {
        pending = pending.with(expectedDelegationGeneration: conflict.currentGeneration)
        try store.savePendingEnrollment(pending)
        if retryStaleGeneration {
          return try await completeEnrollment(
            pending, storedRecords: storedRecords, relayMetadataPubkey: relayMetadataPubkey,
            retryStaleGeneration: false
          )
        }
      }
      throw error
    }

    let record = BuzzPushEndpointGrantRecord(
      gatewayOrigin: gatewayOrigin,
      relayOrigin: pending.relayOrigin,
      relayPubkey: relayPubkey,
      relayMetadataPubkey: relayMetadataPubkey,
      gatewayInstallationHandle: installationHandle,
      appAttestKeyId: appAttestKeyId,
      installationId: pending.installationId,
      endpointGrant: endpointGrant,
      endpointHash: pending.endpointHash,
      endpointEpoch: endpointEpoch,
      generation: generation,
      expiresAt: pending.expiresAt
    )
    try await retireReplacedDelegation(relayOrigin: record.relayOrigin,
      installationHandle: record.gatewayInstallationHandle, relayPubkey: record.relayPubkey)
    try store.save(record)
    try store.removePendingEnrollment(
      gatewayOrigin: gatewayOrigin,
      relayOrigin: pending.relayOrigin
    )
    return record
  }

  private func retireReplacedDelegation(
    relayOrigin: String, installationHandle: String?, relayPubkey: String
  ) async throws {
    let records = try store.records()
    guard let old = records.first(where: {
      $0.gatewayOrigin == gatewayOrigin && $0.relayOrigin == relayOrigin
    }), old.gatewayInstallationHandle != installationHandle || old.relayPubkey != relayPubkey,
      let handle = old.gatewayInstallationHandle, let installation = UUID(uuidString: handle)
    else { return }
    guard !records.contains(where: {
      $0.gatewayOrigin == old.gatewayOrigin && $0.relayOrigin != old.relayOrigin
        && $0.gatewayInstallationHandle == handle && $0.relayPubkey == old.relayPubkey
        && $0.expiresAt > Int64(now().timeIntervalSince1970) && !$0.endpointGrant.isEmpty
    }) else { return }
    // Keep the old target durable until its revocation succeeds; replacing the
    // origin record first would lose it even if the new issuance is journaled.
    try await revokeDelegation(installationHandle: installation, keyId: old.appAttestKeyId,
      relayPubkey: old.relayPubkey, generation: old.generation)
  }

  private func makeInstallationId() throws -> String {
    let bytes = try installationIdBytes()
    precondition(
      bytes.count == 16,
      "NIP-PL installation identity entropy must be exactly 16 bytes"
    )
    // This value is per relay origin and never leaves the relay-facing lease.
    return Self.lowercaseHex(bytes)
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
    requestExpiresAt: Int64,
    installationHandle: UUID,
    endpointEpoch: Int64,
    relayPubkey: String,
    expectedGeneration: Int64,
    generation: Int64,
    notBefore: Int64,
    expiresAt: Int64,
    assertion: String
  ) async throws -> String {
    let response: DelegationResponse = try await post(
      route: "v1/delegations",
      expectedStatus: 201,
      body: DelegationRequest(
        v: 2,
        gatewayOrigin: gatewayOrigin,
        requestExpiresAt: requestExpiresAt,
        installationHandle: installationHandle.uuidString.lowercased(),
        endpointEpoch: endpointEpoch,
        expectedGeneration: expectedGeneration,
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

  private func fetchCurrentRelayKeys(from relayOrigin: URL) async throws -> RelayKeys {
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
    guard current.count == 1,
      Self.isLowercaseHexPubkey(current[0].pubkey)
    else {
      throw BuzzDevPushEnrollmentError.invalidRelayDescriptor
    }
    let metadataPubkey = document.relaySelf.flatMap {
      Self.isLowercaseHexPubkey($0) ? $0 : nil
    }
    return RelayKeys(
      pushPubkey: current[0].pubkey,
      metadataPubkey: metadataPubkey
    )
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

  private static func endpointHash(_ endpoint: String) -> String? {
    let utf8 = Array(endpoint.utf8)
    guard !utf8.isEmpty, utf8.count <= 512, utf8.count.isMultiple(of: 2) else {
      return nil
    }
    var bytes = [UInt8]()
    bytes.reserveCapacity(utf8.count / 2)
    for index in stride(from: 0, to: utf8.count, by: 2) {
      guard let high = hexNibble(utf8[index]), let low = hexNibble(utf8[index + 1]) else {
        return nil
      }
      bytes.append(high << 4 | low)
    }
    return lowercaseHex(Data(SHA256.hash(data: Data(bytes))))
  }

  private static func hexNibble(_ byte: UInt8) -> UInt8? {
    switch byte {
    case 48...57: byte - 48
    case 97...102: byte - 87
    default: nil
    }
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
  let endpoint: String
  let endpointEpoch: Int64
  let expiresAt: Int64
  enum CodingKeys: String, CodingKey {
    case v
    case challengeId = "challenge_id"
    case challenge
    case keyId = "key_id"
    case attestation
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
  let gatewayOrigin: String
  let requestExpiresAt: Int64
  let installationHandle: String
  let endpointEpoch: Int64
  let expectedGeneration: Int64
  let generation: Int64
  let relayPubkey: String
  let notBefore: Int64
  let expiresAt: Int64
  let assertion: String
  enum CodingKeys: String, CodingKey {
    case v
    case gatewayOrigin = "gateway_origin"
    case requestExpiresAt = "request_expires_at"
    case installationHandle = "installation_handle"
    case endpointEpoch = "endpoint_epoch"
    case expectedGeneration = "expected_generation"
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
private struct RotateEndpointRequest: Encodable {
  let v: Int
  let gatewayOrigin: String
  let requestExpiresAt: Int64
  let installationHandle: String
  let endpointEpoch: Int64
  let newEndpointEpoch: Int64
  let endpoint: String
  let assertion: String
  enum CodingKeys: String, CodingKey {
    case v, endpoint, assertion
    case gatewayOrigin = "gateway_origin"
    case requestExpiresAt = "request_expires_at"
    case installationHandle = "installation_handle"
    case endpointEpoch = "endpoint_epoch"
    case newEndpointEpoch = "new_endpoint_epoch"
  }
}
private struct RevokeDelegationRequest: Encodable {
  let v: Int
  let gatewayOrigin: String
  let requestExpiresAt: Int64
  let installationHandle: String
  let relayPubkey: String
  let generation: Int64
  let assertion: String
  enum CodingKeys: String, CodingKey {
    case v
    case gatewayOrigin = "gateway_origin"
    case requestExpiresAt = "request_expires_at"
    case installationHandle = "installation_handle"
    case relayPubkey = "relay_pubkey"
    case generation, assertion
  }
}
private struct MutationResponse: Decodable { let status: String }
private struct DelegationConflict: Decodable {
  let error: String
  let currentGeneration: Int64
  enum CodingKeys: String, CodingKey {
    case error
    case currentGeneration = "current_generation"
  }
}
private struct RelayInformation: Decodable {
  struct Push: Decodable {
    struct Key: Decodable {
      let pubkey: String
      let current: Bool
    }
    let keys: [Key]
  }
  let relaySelf: String?
  let push: Push

  enum CodingKeys: String, CodingKey {
    case relaySelf = "self"
    case push
  }

  init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    push = try container.decode(Push.self, forKey: .push)
    relaySelf = try? container.decode(String.self, forKey: .relaySelf)
  }
}

private struct RelayKeys {
  let pushPubkey: String
  let metadataPubkey: String?
}

extension BuzzPushPendingEnrollmentRecord {
  /// This journal with only its replacement bookkeeping changed.
  fileprivate func with(
    preparedAt: Int64? = nil, rejectedReplacements: Int? = nil,
    expectedDelegationGeneration: Int64? = nil
  ) -> Self {
    Self(
      gatewayOrigin: gatewayOrigin,
      relayOrigin: relayOrigin,
      relayPubkey: relayPubkey,
      endpoint: endpoint,
      endpointHash: endpointHash,
      endpointEpoch: endpointEpoch,
      rotationFromEpoch: rotationFromEpoch,
      expiresAt: expiresAt,
      installationId: installationId,
      gatewayInstallationHandle: gatewayInstallationHandle,
      challengeId: challengeId,
      challenge: challenge,
      keyId: keyId,
      attestation: attestation,
      delegationGeneration: delegationGeneration,
      expectedDelegationGeneration: expectedDelegationGeneration ?? self.expectedDelegationGeneration,
      preparedAt: preparedAt ?? self.preparedAt,
      rejectedReplacements: rejectedReplacements ?? self.rejectedReplacements
    )
  }
}

private extension BuzzPushEndpointGrantRecord {
  func with(endpointGrant: String, endpointHash: String? = nil, endpointEpoch: Int64? = nil) -> Self {
    Self(
      gatewayOrigin: gatewayOrigin, relayOrigin: relayOrigin, relayPubkey: relayPubkey,
      relayMetadataPubkey: relayMetadataPubkey, gatewayInstallationHandle: gatewayInstallationHandle,
      appAttestKeyId: appAttestKeyId, installationId: installationId,
      endpointGrant: endpointGrant, endpointHash: endpointHash ?? self.endpointHash,
      endpointEpoch: endpointEpoch ?? self.endpointEpoch, generation: generation, expiresAt: expiresAt
    )
  }
}
