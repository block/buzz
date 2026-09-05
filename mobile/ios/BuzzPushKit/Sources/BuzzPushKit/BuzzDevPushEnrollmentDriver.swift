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
  public let installationId: String
  public let endpointGrant: String
  public let endpointHash: String
  public let appProfile: String
  public let endpointEpoch: Int64
  public let generation: Int64
  public let expiresAt: Int64

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
    appProfile: String,
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
    self.appProfile = appProfile
    self.endpointEpoch = endpointEpoch
    self.generation = generation
    self.expiresAt = expiresAt
  }
}

/// Durable state retained while installations from an earlier gateway are revoked.
public struct BuzzPushGatewayCleanupState: Codable, Equatable, Sendable {
  /// Gateway whose installation authority must be revoked.
  public let gatewayOrigin: String
  /// Grants that retain installation handles and endpoint epochs for revocation.
  public var grants: [BuzzPushEndpointGrantRecord]
  /// Crash-recovery journals, including response-loss enrollments that may need replay.
  public var pendingEnrollments: [BuzzPushPendingEnrollmentRecord]
  /// Handles whose revocation intent was persisted before the remote mutation.
  /// These records must never be restored as usable grants after a gateway rollback.
  public var revocationPendingInstallationHandles: [String]?

  /// Creates one durable cleanup snapshot for a retired gateway.
  public init(
    gatewayOrigin: String,
    grants: [BuzzPushEndpointGrantRecord],
    pendingEnrollments: [BuzzPushPendingEnrollmentRecord],
    revocationPendingInstallationHandles: [String]? = nil
  ) {
    self.gatewayOrigin = gatewayOrigin
    self.grants = grants
    self.pendingEnrollments = pendingEnrollments
    self.revocationPendingInstallationHandles = revocationPendingInstallationHandles
  }
}

/// Durable same-gateway lease replacement inventory with a compare-and-swap
/// generation that fences checkpoints from newer endpoint mutations.
public struct BuzzPushReplacementQueueState: Codable, Equatable, Sendable {
  public let generation: Int64
  public var relayOrigins: [String]

  public init(generation: Int64, relayOrigins: [String]) {
    self.generation = generation
    self.relayOrigins = relayOrigins
  }
}

/// Persistence boundary for endpoint grants. The Runner implementation stores
/// records in its Keychain access group and exposes them over the Flutter bridge.
public protocol BuzzPushEndpointGrantStore {
  /// Moves records from other gateways into the cleanup journal.
  func reset(forGatewayOrigin gatewayOrigin: String) throws
  func records() throws -> [BuzzPushEndpointGrantRecord]
  func save(_ record: BuzzPushEndpointGrantRecord) throws
  /// Atomically removes one relay-origin grant without touching sibling origins.
  func removeRecord(gatewayOrigin: String, relayOrigin: String, appProfile: String) throws
  /// Atomically removes every active grant backed by one installation.
  func removeRecords(gatewayOrigin: String, installationHandle: String) throws
  /// Atomically removes every active grant backed by one delegation.
  func removeRecords(
    gatewayOrigin: String,
    installationHandle: String,
    relayPubkey: String
  ) throws
  func pendingEnrollment(
    gatewayOrigin: String,
    relayOrigin: String,
    appProfile: String
  ) throws -> BuzzPushPendingEnrollmentRecord?
  func savePendingEnrollment(_ record: BuzzPushPendingEnrollmentRecord) throws
  func removePendingEnrollment(
    gatewayOrigin: String,
    relayOrigin: String,
    appProfile: String
  ) throws
  /// Returns every retired-gateway cleanup snapshot.
  func gatewayCleanupStates() throws -> [BuzzPushGatewayCleanupState]
  /// Atomically replaces one retired-gateway cleanup snapshot.
  func saveGatewayCleanupState(_ state: BuzzPushGatewayCleanupState) throws
  /// Deletes a cleanup snapshot only after its installations are terminal.
  func removeGatewayCleanupState(gatewayOrigin: String) throws
  /// Relay origins whose leases must be republished after shared installation
  /// authority was revoked. The queue is persisted before the remote mutation.
  func replacementQueueState() throws -> BuzzPushReplacementQueueState
  /// Atomically merges relay origins into the durable replacement queue.
  func queueReplacementRelayOrigins(_ relayOrigins: [String]) throws
  /// Atomically removes relay origins sharing delegation authority after all
  /// of their community leases are durable.
  func checkpointReplacementRelayOrigins(
    _ relayOrigins: [String],
    expectedGeneration: Int64
  ) throws -> Bool
  /// Clears the queue only after replacement publication has completed.
  func clearReplacementRelayOrigins() throws
  /// Opaque grants retained from the gateway-neutral legacy schema. A gateway
  /// may open one as proof that it owns the conflicting installation.
  func quarantinedLegacyEndpointGrants() throws -> [String]
  func quarantinedLegacyPendingEnrollments() throws
    -> [BuzzPushLegacyRecoveryInventory.BuzzPushLegacyPendingRecovery]
}

extension BuzzPushEndpointGrantStore {
  public func quarantinedLegacyEndpointGrants() throws -> [String] { [] }
  public func quarantinedLegacyPendingEnrollments() throws
    -> [BuzzPushLegacyRecoveryInventory.BuzzPushLegacyPendingRecovery]
  { [] }
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
  case retiredGatewayCleanupIncomplete

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
    case .retiredGatewayCleanupIncomplete:
      return "A retired push gateway installation could not be revoked yet."
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
  public static let appProfile = "buzz-ios-dogfood"
  public static let endpointEpoch: Int64 = 1

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
    resetStore: Bool = true,
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
    if resetStore {
      try store.reset(forGatewayOrigin: canonical.text)
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

  public func endpointGrants() throws -> [BuzzPushEndpointGrantRecord] {
    try store.records()
  }

  /// Fetches the relay's current NIP-11 push key, enrolls the APNs endpoint,
  /// delegates to that key, and durably saves the resulting opaque grant.
  public func enroll(
    deviceToken: Data,
    relayURL: URL,
    forceDelegationRenewal: Bool = false
  ) async throws -> BuzzPushEndpointGrantRecord {
    return try await enrollCurrent(
      deviceToken: deviceToken,
      relayURL: relayURL,
      forceDelegationRenewal: forceDelegationRenewal
    )
  }

  /// Revokes durable installations from gateways that are no longer configured.
  public func cleanRetiredGateways(deviceToken: Data? = nil) async throws {
    precondition(deviceToken?.isEmpty != true, "The APNs device token must not be empty")
    try await cleanStaleGateways(deviceToken: deviceToken)
  }

  private func enrollCurrent(
    deviceToken: Data,
    relayURL: URL,
    forceDelegationRenewal: Bool
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
        && $0.appProfile == Self.appProfile
    }
    var pendingEnrollment = try store.pendingEnrollment(
      gatewayOrigin: gatewayOrigin,
      relayOrigin: relayOrigin.text,
      appProfile: Self.appProfile
    )
    if let pending = pendingEnrollment,
      pending.relayPubkey != relayPubkey || pending.endpointHash != endpointHash
        || pending.expiresAt <= nowSeconds
    {
      var revokedInstallation = false
      let referencedInstallation = pending.gatewayInstallationHandle.flatMap { handle in
        storedRecords
          .filter {
            $0.gatewayOrigin == pending.gatewayOrigin
              && $0.gatewayInstallationHandle == handle
              && $0.relayPubkey == pending.relayPubkey
              && $0.appProfile == pending.appProfile
              && $0.expiresAt > nowSeconds
          }
          .max { $0.generation < $1.generation }
      }
      if pending.delegationRevoked == true,
        let handleText = pending.gatewayInstallationHandle,
        let handle = UUID(uuidString: handleText),
        handleText == handle.uuidString.lowercased()
      {
        try store.removeRecords(
          gatewayOrigin: gatewayOrigin,
          installationHandle: handleText,
          relayPubkey: pending.relayPubkey
        )
      } else if let handleText = pending.gatewayInstallationHandle,
        let handle = UUID(uuidString: handleText),
        handleText == handle.uuidString.lowercased(),
        let keyId = pending.keyId,
        pending.endpointHash == endpointHash,
        let referencedInstallation
      {
        let siblingDelegationRecords = storedRecords.filter {
          $0.gatewayOrigin == gatewayOrigin
            && $0.gatewayInstallationHandle == handleText
            && $0.relayPubkey == pending.relayPubkey
            && $0.appProfile == pending.appProfile
            && $0.relayOrigin != pending.relayOrigin
        }
        if !siblingDelegationRecords.isEmpty {
          // Delegation authority is shared by relay key, installation, and app
          // profile. A response-lost higher generation may already have
          // invalidated every sibling grant, so queue their relay origins
          // before discarding the journal. Keep the delegation alive and
          // remove only the rotating origin's obsolete grant.
          try store.queueReplacementRelayOrigins(
            siblingDelegationRecords.map(\.relayOrigin)
          )
          try store.removeRecord(
            gatewayOrigin: gatewayOrigin,
            relayOrigin: pending.relayOrigin,
            appProfile: pending.appProfile
          )
        } else {
          let candidateGenerations = [
            pending.delegationGeneration,
            referencedInstallation.generation,
          ].filter { $0 > 0 }.reduce(into: [Int64]()) { generations, generation in
            if !generations.contains(generation) { generations.append(generation) }
          }
          var revoked = false
          for generation in candidateGenerations {
            do {
              try await revokeDelegation(
                installationHandle: handle,
                relayPubkey: pending.relayPubkey,
                generation: generation,
                appAttestKeyId: keyId
              )
              revoked = true
              break
            } catch BuzzDevPushEnrollmentError.unexpectedStatus(
              route: "v1/delegations/revoke", _, actual: 404, _
            ) {
              // The reserved generation may not have committed. Try the last
              // generation known to have produced a durable grant as well.
            }
          }
          guard revoked else {
            throw BuzzDevPushEnrollmentError.retiredGatewayCleanupIncomplete
          }
          try store.savePendingEnrollment(pending.withDelegationRevoked())
          try store.removeRecords(
            gatewayOrigin: gatewayOrigin,
            installationHandle: handleText,
            relayPubkey: pending.relayPubkey
          )
        }
      } else {
        let replacementRelayOrigins: [String] = storedRecords.compactMap { record in
          guard record.gatewayOrigin == gatewayOrigin,
            record.gatewayInstallationHandle == pending.gatewayInstallationHandle
          else { return nil }
          return record.relayOrigin
        }
        // Revoking an installation invalidates every lease backed by its
        // grants. Queue those relay origins before either local or remote
        // authority is removed so a crash cannot strand sibling communities.
        try store.queueReplacementRelayOrigins(replacementRelayOrigins)
        var cleanupState =
          try store.gatewayCleanupStates().first {
            $0.gatewayOrigin == gatewayOrigin
          }
          ?? BuzzPushGatewayCleanupState(
            gatewayOrigin: gatewayOrigin,
            grants: [],
            pendingEnrollments: []
          )
        cleanupState.pendingEnrollments.removeAll {
          $0.relayOrigin == pending.relayOrigin && $0.appProfile == pending.appProfile
        }
        cleanupState.pendingEnrollments.append(pending)
        if let installationHandle = pending.gatewayInstallationHandle {
          for record in storedRecords
          where record.gatewayOrigin == gatewayOrigin
            && record.gatewayInstallationHandle == installationHandle
          {
            cleanupState.grants.removeAll {
              $0.relayOrigin == record.relayOrigin && $0.appProfile == record.appProfile
            }
            cleanupState.grants.append(record)
          }
          try store.saveGatewayCleanupState(cleanupState)
          try store.removeRecords(
            gatewayOrigin: gatewayOrigin,
            installationHandle: installationHandle
          )
        }
        guard await cleanStaleGateway(&cleanupState, deviceToken: deviceToken) else {
          if let reconciled = cleanupState.pendingEnrollments.first(where: {
            $0.relayOrigin == pending.relayOrigin && $0.appProfile == pending.appProfile
          }) {
            try store.savePendingEnrollment(reconciled)
          }
          throw BuzzDevPushEnrollmentError.retiredGatewayCleanupIncomplete
        }
        try store.removeGatewayCleanupState(gatewayOrigin: gatewayOrigin)
        revokedInstallation = true
      }
      try store.removePendingEnrollment(
        gatewayOrigin: gatewayOrigin,
        relayOrigin: relayOrigin.text,
        appProfile: Self.appProfile
      )
      pendingEnrollment = nil
      if revokedInstallation {
        return try await enrollCurrent(
          deviceToken: deviceToken,
          relayURL: relayURL,
          forceDelegationRenewal: forceDelegationRenewal
        )
      }
    }
    let reusableCurrent = storedForOrigin.flatMap { current in
      current.relayPubkey == relayPubkey
        && current.endpointHash == endpointHash
        && current.endpointEpoch == Self.endpointEpoch
        && current.expiresAt > nowSeconds + 300 ? current : nil
    }
    let newestSharedGrant = storedRecords.filter {
      $0.gatewayOrigin == gatewayOrigin && $0.relayPubkey == relayPubkey
        && $0.appProfile == Self.appProfile
        && $0.endpointHash == endpointHash && $0.endpointEpoch == Self.endpointEpoch
        && $0.expiresAt > nowSeconds + 300
    }.max { $0.generation < $1.generation }
    let newerSiblingGrant = reusableCurrent.flatMap { current in
      newestSharedGrant.flatMap { shared in
        shared.generation > current.generation ? shared : nil
      }
    }
    if let reusableGrant = forceDelegationRenewal ? newerSiblingGrant : newestSharedGrant {
      // Delegation authority is shared by installation and relay key. When a
      // sibling origin has already renewed it, adopt that newest opaque grant
      // even if this origin is still durably queued for forced replacement.
      let record = BuzzPushEndpointGrantRecord(
        gatewayOrigin: gatewayOrigin,
        relayOrigin: relayOrigin.text,
        relayPubkey: relayPubkey,
        relayMetadataPubkey: relayKeys.metadataPubkey,
        gatewayInstallationHandle: reusableGrant.gatewayInstallationHandle,
        appAttestKeyId: reusableGrant.appAttestKeyId,
        installationId: try reusableCurrent?.installationId ?? makeInstallationId(),
        endpointGrant: reusableGrant.endpointGrant,
        endpointHash: endpointHash,
        appProfile: Self.appProfile,
        endpointEpoch: reusableGrant.endpointEpoch,
        generation: reusableGrant.generation,
        expiresAt: reusableGrant.expiresAt
      )
      try store.save(record)
      try store.removePendingEnrollment(
        gatewayOrigin: gatewayOrigin,
        relayOrigin: relayOrigin.text,
        appProfile: Self.appProfile
      )
      return record
    }

    // A previously attested installation can delegate independently to a new
    // relay key, or issue a higher-generation grant for the same relay,
    // without attempting duplicate APNs-token enrollment. An installation in
    // its final five minutes is renewed by the authenticated delegation.
    let reusableInstallation = storedRecords.first { record in
      guard record.gatewayOrigin == gatewayOrigin,
        record.appProfile == Self.appProfile,
        record.endpointHash == endpointHash,
        record.endpointEpoch == Self.endpointEpoch,
        record.expiresAt > nowSeconds,
        let handle = record.gatewayInstallationHandle,
        let uuid = UUID(uuidString: handle)
      else { return false }
      return handle == uuid.uuidString.lowercased()
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
      pending = BuzzPushPendingEnrollmentRecord(
        gatewayOrigin: gatewayOrigin,
        relayOrigin: relayOrigin.text,
        relayPubkey: relayPubkey,
        endpoint: endpoint,
        endpointHash: endpointHash,
        appProfile: Self.appProfile,
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
      pending = BuzzPushPendingEnrollmentRecord(
        gatewayOrigin: gatewayOrigin,
        relayOrigin: relayOrigin.text,
        relayPubkey: relayPubkey,
        endpoint: endpoint,
        endpointHash: endpointHash,
        appProfile: Self.appProfile,
        expiresAt: expiresAt,
        installationId: try storedForOrigin?.installationId ?? makeInstallationId(),
        challengeId: enrollmentChallenge.id.uuidString.lowercased(),
        challenge: enrollmentChallenge.value,
        keyId: attestation.keyId,
        attestation: attestation.attestation
      )
      // The exact signed request is durable before the first network attempt.
      try store.savePendingEnrollment(pending)
    }

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
      } catch BuzzDevPushEnrollmentError.unexpectedStatus(
        route: "v1/installations", _, actual: 404, _
      ) where pendingEnrollment != nil {
        // No installation was committed and the original challenge expired.
        // Discard the prepared request and start once with a fresh App Attest key.
        try store.removePendingEnrollment(
          gatewayOrigin: gatewayOrigin,
          relayOrigin: relayOrigin.text,
          appProfile: Self.appProfile
        )
        return try await enrollCurrent(
          deviceToken: deviceToken,
          relayURL: relayURL,
          forceDelegationRenewal: forceDelegationRenewal
        )
      } catch let error as BuzzDevPushEnrollmentError {
        guard
          case .unexpectedStatus(
            route: "v1/installations", _, actual: 409, _
          ) = error
        else { throw error }
        let cleanupStates = try store.gatewayCleanupStates()
        if cleanupStates.isEmpty {
          let recovered = try await recoverQuarantinedLegacyInstallation(endpoint: endpoint)
          guard recovered else { throw error }
          try store.removePendingEnrollment(
            gatewayOrigin: gatewayOrigin,
            relayOrigin: relayOrigin.text,
            appProfile: Self.appProfile
          )
          return try await enrollCurrent(
            deviceToken: deviceToken,
            relayURL: relayURL,
            forceDelegationRenewal: forceDelegationRenewal
          )
        }
        let affectedRelayOrigins = cleanupStates.flatMap {
          $0.grants.map(\.relayOrigin) + $0.pendingEnrollments.map(\.relayOrigin)
        }
        // A gateway origin can change while retaining the same backing
        // authority store. Its live installation then conflicts with the new
        // origin's enrollment. Preserve replacement work before retiring that
        // authority, then retry against the released APNs token.
        try store.queueReplacementRelayOrigins(affectedRelayOrigins)
        try await cleanStaleGateways(deviceToken: deviceToken)
        try store.removePendingEnrollment(
          gatewayOrigin: gatewayOrigin,
          relayOrigin: relayOrigin.text,
          appProfile: Self.appProfile
        )
        return try await enrollCurrent(
          deviceToken: deviceToken,
          relayURL: relayURL,
          forceDelegationRenewal: forceDelegationRenewal
        )
      }
      pending = BuzzPushPendingEnrollmentRecord(
        gatewayOrigin: gatewayOrigin,
        relayOrigin: pending.relayOrigin,
        relayPubkey: pending.relayPubkey,
        endpoint: pending.endpoint,
        endpointHash: pending.endpointHash,
        appProfile: pending.appProfile,
        expiresAt: pending.expiresAt,
        installationId: pending.installationId,
        gatewayInstallationHandle: installation.uuidString.lowercased(),
        challengeId: pending.challengeId,
        challenge: pending.challenge,
        keyId: pending.keyId,
        attestation: pending.attestation,
        delegationGeneration: pending.delegationGeneration,
        delegationRevoked: pending.delegationRevoked
      )
      try store.savePendingEnrollment(pending)
    }

    let installationHandle = installation.uuidString.lowercased()
    let currentGeneration =
      storedRecords
      .filter {
        $0.gatewayInstallationHandle == installationHandle
          && $0.relayPubkey == relayPubkey && $0.appProfile == Self.appProfile
      }
      .map(\.generation)
      .max()
    let generationBase = max(currentGeneration ?? 0, pending.delegationGeneration)
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
      appProfile: pending.appProfile,
      expiresAt: pending.expiresAt,
      installationId: pending.installationId,
      gatewayInstallationHandle: installationHandle,
      challengeId: pending.challengeId,
      challenge: pending.challenge,
      keyId: pending.keyId,
      attestation: pending.attestation,
      delegationGeneration: generation
    )
    // Reserve before delegation so a committed delegation followed by a local
    // save failure is retried at a strictly higher generation.
    try store.savePendingEnrollment(pending)

    let delegationChallenge = try await challenge()
    let delegationClientData = try BuzzPushTranscript.delegate(
      gatewayOrigin: gatewayBaseURL,
      challengeId: delegationChallenge.id,
      challenge: delegationChallenge.value,
      installationHandle: installation,
      endpointEpoch: Self.endpointEpoch,
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
    let endpointGrant = try await delegate(
      challenge: delegationChallenge,
      installationHandle: installation,
      relayPubkey: relayPubkey,
      generation: generation,
      notBefore: nowSeconds,
      expiresAt: pending.expiresAt,
      assertion: assertion
    )

    let record = BuzzPushEndpointGrantRecord(
      gatewayOrigin: gatewayOrigin,
      relayOrigin: relayOrigin.text,
      relayPubkey: relayPubkey,
      relayMetadataPubkey: relayKeys.metadataPubkey,
      gatewayInstallationHandle: installationHandle,
      appAttestKeyId: appAttestKeyId,
      installationId: pending.installationId,
      endpointGrant: endpointGrant,
      endpointHash: endpointHash,
      appProfile: Self.appProfile,
      endpointEpoch: Self.endpointEpoch,
      generation: generation,
      expiresAt: pending.expiresAt
    )
    try store.save(record)
    try store.removePendingEnrollment(
      gatewayOrigin: gatewayOrigin,
      relayOrigin: relayOrigin.text,
      appProfile: Self.appProfile
    )
    return record
  }

  private func cleanStaleGateways(deviceToken: Data?) async throws {
    let states = try store.gatewayCleanupStates()
    var cleanupIncomplete = false
    var persistenceError: Error?
    for var state in states {
      guard await cleanStaleGateway(&state, deviceToken: deviceToken) else {
        cleanupIncomplete = true
        continue
      }
      do {
        try store.removeGatewayCleanupState(gatewayOrigin: state.gatewayOrigin)
      } catch {
        if persistenceError == nil { persistenceError = error }
      }
    }
    if let persistenceError { throw persistenceError }
    if cleanupIncomplete {
      throw BuzzDevPushEnrollmentError.retiredGatewayCleanupIncomplete
    }
  }

  private func cleanStaleGateway(
    _ state: inout BuzzPushGatewayCleanupState,
    deviceToken: Data?
  ) async -> Bool {
    guard let oldURL = URL(string: state.gatewayOrigin),
      let oldDriver = try? BuzzDevPushEnrollmentDriver(
        gatewayBaseURL: oldURL,
        store: store,
        session: session,
        appAttest: appAttest,
        now: now,
        lifetimeSeconds: lifetimeSeconds,
        resetStore: false,
        installationIdBytes: installationIdBytes
      )
    else { return false }
    let nowSeconds = Int64(now().timeIntervalSince1970)
    let currentEndpoint = deviceToken.map(Self.lowercaseHex)
    let currentEndpointHash = deviceToken.map {
      Self.lowercaseHex(Data(SHA256.hash(data: $0)))
    }
    var handles = [String: CleanupInstallation]()
    func mergeHandle(_ handle: String, endpointEpoch: Int64, keyId: String) -> Bool {
      if let existing = handles[handle] {
        guard existing.keyId == keyId else { return false }
        handles[handle] = CleanupInstallation(
          endpointEpoch: max(existing.endpointEpoch, endpointEpoch),
          keyId: keyId
        )
      } else {
        handles[handle] = CleanupInstallation(endpointEpoch: endpointEpoch, keyId: keyId)
      }
      return true
    }
    for grant in state.grants {
      if grant.expiresAt <= nowSeconds { continue }
      guard let handle = grant.gatewayInstallationHandle else { return false }
      guard
        mergeHandle(
          handle,
          endpointEpoch: grant.endpointEpoch,
          keyId: grant.appAttestKeyId
        )
      else { return false }
    }
    for index in state.pendingEnrollments.indices {
      var pending = state.pendingEnrollments[index]
      if pending.expiresAt <= nowSeconds { continue }
      if pending.gatewayInstallationHandle == nil {
        let replayEndpoint: String
        if let protectedEndpoint = pending.endpoint {
          guard Self.endpointHash(protectedEndpoint) == pending.endpointHash else { return false }
          replayEndpoint = protectedEndpoint
        } else if let currentEndpoint, pending.endpointHash == currentEndpointHash {
          replayEndpoint = currentEndpoint
        } else {
          guard currentEndpoint != nil else { return false }
          // Pre-endpoint journals cannot be replayed after token rotation. They
          // have no known handle to revoke, so this cleanup item is terminal.
          continue
        }
        guard let challengeId = pending.challengeId,
          let challengeUUID = UUID(uuidString: challengeId),
          challengeId == challengeUUID.uuidString.lowercased(),
          let challenge = pending.challenge,
          let keyId = pending.keyId,
          let attestation = pending.attestation
        else { return false }
        do {
          let installation = try await oldDriver.enrollInstallation(
            challenge: Challenge(id: challengeUUID, value: challenge),
            endpoint: replayEndpoint,
            expiresAt: pending.expiresAt,
            attestation: BuzzDevAttestation(keyId: keyId, attestation: attestation)
          )
          pending = pending.withGatewayInstallationHandle(
            installation.uuidString.lowercased()
          )
          state.pendingEnrollments[index] = pending
          try store.saveGatewayCleanupState(state)
        } catch BuzzDevPushEnrollmentError.unexpectedStatus(
          route: "v1/installations", _, actual: 404, _
        ) {
          continue
        } catch {
          return false
        }
      }
      guard let handle = pending.gatewayInstallationHandle else { return false }
      guard let keyId = pending.keyId,
        mergeHandle(handle, endpointEpoch: Self.endpointEpoch, keyId: keyId)
      else {
        return false
      }
    }
    for handleText in handles.keys.sorted() {
      guard let installation = handles[handleText] else { return false }
      guard let handle = UUID(uuidString: handleText) else { return false }
      if state.revocationPendingInstallationHandles?.contains(handleText) != true {
        var pendingHandles = state.revocationPendingInstallationHandles ?? []
        pendingHandles.append(handleText)
        pendingHandles.sort()
        state.revocationPendingInstallationHandles = pendingHandles
        do {
          try store.saveGatewayCleanupState(state)
        } catch {
          return false
        }
      }
      do {
        try await oldDriver.revokeInstallation(
          installationHandle: handle,
          endpointEpoch: installation.endpointEpoch,
          appAttestKeyId: installation.keyId
        )
      } catch {
        return false
      }
      state.grants.removeAll { $0.gatewayInstallationHandle == handleText }
      state.pendingEnrollments.removeAll { $0.gatewayInstallationHandle == handleText }
      state.revocationPendingInstallationHandles?.removeAll { $0 == handleText }
      if state.revocationPendingInstallationHandles?.isEmpty == true {
        state.revocationPendingInstallationHandles = nil
      }
      do {
        try store.saveGatewayCleanupState(state)
      } catch {
        return false
      }
    }
    return true
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

  private func recoverQuarantinedLegacyInstallation(endpoint: String) async throws -> Bool {
    for endpointGrant in try store.quarantinedLegacyEndpointGrants() {
      do {
        let response: MutationResponse = try await post(
          route: "v1/installations/recover",
          expectedStatus: 200,
          body: RecoverInstallationRequest(
            v: 1,
            endpointGrant: endpointGrant,
            appProfile: Self.appProfile,
            endpoint: endpoint
          )
        )
        guard response.status == "revoked" else {
          throw BuzzDevPushEnrollmentError.invalidResponse(
            route: "v1/installations/recover"
          )
        }
        return true
      } catch BuzzDevPushEnrollmentError.unexpectedStatus(
        route: "v1/installations/recover", _, actual: 404, _
      ) {
        continue
      }
    }
    let endpointHash = Self.endpointHash(endpoint)
    for pending in try store.quarantinedLegacyPendingEnrollments() {
      guard pending.appProfile == Self.appProfile,
        pending.endpointHash == endpointHash,
        pending.expiresAt > Int64(now().timeIntervalSince1970),
        let keyId = pending.keyId,
        BuzzAppAttestKeyId.isValid(keyId)
      else { continue }
      do {
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
            let challenge = pending.challenge,
            Self.isBase64URLChallenge(challenge),
            let attestation = pending.attestation,
            !attestation.isEmpty,
            attestation.utf8.count <= 24_000
          else { continue }
          installation = try await enrollInstallation(
            challenge: Challenge(id: challengeUUID, value: challenge),
            endpoint: endpoint,
            expiresAt: pending.expiresAt,
            attestation: BuzzDevAttestation(keyId: keyId, attestation: attestation)
          )
        }
        try await revokeInstallation(
          installationHandle: installation,
          endpointEpoch: Self.endpointEpoch,
          appAttestKeyId: keyId
        )
        return true
      } catch BuzzDevPushEnrollmentError.unexpectedStatus(_, _, actual: 401, _),
        BuzzDevPushEnrollmentError.unexpectedStatus(_, _, actual: 404, _),
        BuzzDevPushEnrollmentError.unexpectedStatus(_, _, actual: 409, _)
      {
        continue
      }
    }
    return false
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

  private func revokeInstallation(
    installationHandle: UUID,
    endpointEpoch: Int64,
    appAttestKeyId: String
  ) async throws {
    let (newEndpointEpoch, overflow) = endpointEpoch.addingReportingOverflow(1)
    guard !overflow else { throw BuzzDevPushEnrollmentError.generationExhausted }
    let revokeChallenge = try await challenge()
    let clientData = try BuzzPushTranscript.revokeInstallation(
      gatewayOrigin: gatewayBaseURL,
      challengeId: revokeChallenge.id,
      challenge: revokeChallenge.value,
      installationHandle: installationHandle,
      endpointEpoch: endpointEpoch,
      newEndpointEpoch: newEndpointEpoch
    )
    let assertion = try await appAttest.assertion(
      keyId: appAttestKeyId,
      clientData: clientData
    )
    let response: MutationResponse = try await post(
      route: "v1/installations/revoke",
      expectedStatus: 200,
      body: RevokeInstallationRequest(
        v: 1,
        challengeId: revokeChallenge.id.uuidString.lowercased(),
        challenge: revokeChallenge.value,
        installationHandle: installationHandle.uuidString.lowercased(),
        endpointEpoch: endpointEpoch,
        newEndpointEpoch: newEndpointEpoch,
        assertion: assertion
      )
    )
    guard response.status == "revoked" else {
      throw BuzzDevPushEnrollmentError.invalidResponse(route: "v1/installations/revoke")
    }
  }

  private func revokeDelegation(
    installationHandle: UUID,
    relayPubkey: String,
    generation: Int64,
    appAttestKeyId: String
  ) async throws {
    let revokeChallenge = try await challenge()
    let clientData = try BuzzPushTranscript.revokeDelegation(
      gatewayOrigin: gatewayBaseURL,
      challengeId: revokeChallenge.id,
      challenge: revokeChallenge.value,
      installationHandle: installationHandle,
      relayPubkey: relayPubkey,
      generation: generation
    )
    let assertion = try await appAttest.assertion(
      keyId: appAttestKeyId,
      clientData: clientData
    )
    let response: MutationResponse = try await post(
      route: "v1/delegations/revoke",
      expectedStatus: 200,
      body: RevokeDelegationRequest(
        v: 1,
        challengeId: revokeChallenge.id.uuidString.lowercased(),
        challenge: revokeChallenge.value,
        installationHandle: installationHandle.uuidString.lowercased(),
        relayPubkey: relayPubkey,
        generation: generation,
        assertion: assertion
      )
    )
    guard response.status == "revoked" else {
      throw BuzzDevPushEnrollmentError.invalidResponse(route: "v1/delegations/revoke")
    }
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
private struct RecoverInstallationRequest: Encodable {
  let v: Int
  let endpointGrant: String
  let appProfile: String
  let endpoint: String
  enum CodingKeys: String, CodingKey {
    case v
    case endpointGrant = "endpoint_grant"
    case appProfile = "app_profile"
    case endpoint
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
private struct RevokeDelegationRequest: Encodable {
  let v: Int
  let challengeId: String
  let challenge: String
  let installationHandle: String
  let relayPubkey: String
  let generation: Int64
  let assertion: String
  enum CodingKeys: String, CodingKey {
    case v
    case challengeId = "challenge_id"
    case challenge
    case installationHandle = "installation_handle"
    case relayPubkey = "relay_pubkey"
    case generation
    case assertion
  }
}
private struct RevokeInstallationRequest: Encodable {
  let v: Int
  let challengeId: String
  let challenge: String
  let installationHandle: String
  let endpointEpoch: Int64
  let newEndpointEpoch: Int64
  let assertion: String
  enum CodingKeys: String, CodingKey {
    case v
    case challengeId = "challenge_id"
    case challenge
    case installationHandle = "installation_handle"
    case endpointEpoch = "endpoint_epoch"
    case newEndpointEpoch = "new_endpoint_epoch"
    case assertion
  }
}
private struct MutationResponse: Decodable {
  let status: String
}
private struct CleanupInstallation {
  let endpointEpoch: Int64
  let keyId: String
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
