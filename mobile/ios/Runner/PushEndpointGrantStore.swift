import BuzzPushKit
import Foundation
import Security

/// Keychain-backed endpoint grant storage. The opaque grant is never written to
/// UserDefaults or logs. Dart can read the closed record through the push bridge.
final class BuzzPushEndpointGrantKeychainStore: BuzzPushEndpointGrantStore {
  private static let service = "buzz.push.endpoint-grants"
  private static let legacyRecordsAccount = "v1"
  private static let legacyPendingAccount = "pending-v1"
  private static let recordsAccount = "v2"
  private static let pendingAccount = "pending-v2"
  private static let cleanupAccount = "gateway-cleanup-v1"
  private static let replacementRelaysAccount = "replacement-relays-v1"

  private let accessGroup: String?

  init(accessGroup: String?) {
    self.accessGroup = accessGroup
  }

  func reset(forGatewayOrigin gatewayOrigin: String) throws {
    let legacyInventory = try quarantinedLegacyInventory()
    if !legacyInventory.relayOrigins.isEmpty {
      try queueReplacementRelayOrigins(legacyInventory.relayOrigins)
    }
    let allRecords = try records()
    let allPending = try pendingEnrollments()
    try BuzzPushGatewayStateReset.run(
      gatewayOrigin: gatewayOrigin,
      records: allRecords,
      pendingEnrollments: allPending,
      cleanupStates: gatewayCleanupStates(),
      saveCleanupState: saveGatewayCleanupState,
      removeCleanupState: removeGatewayCleanupState,
      replaceRecords: { try self.replace($0, account: Self.recordsAccount) },
      replacePendingEnrollments: { try self.replace($0, account: Self.pendingAccount) }
    )
  }

  func quarantinedLegacyEndpointGrants() throws -> [String] {
    try quarantinedLegacyInventory().endpointGrants
  }

  func quarantinedLegacyPendingEnrollments() throws
    -> [BuzzPushLegacyRecoveryInventory.BuzzPushLegacyPendingRecovery]
  {
    try quarantinedLegacyInventory().pendingEnrollments
  }

  func clearQuarantinedLegacyState() throws {
    try delete(account: Self.legacyRecordsAccount)
    try delete(account: Self.legacyPendingAccount)
  }

  private func quarantinedLegacyInventory() throws -> BuzzPushLegacyRecoveryInventory {
    try BuzzPushLegacyRecoveryInventory.decode(
      grants: data(account: Self.legacyRecordsAccount),
      pending: data(account: Self.legacyPendingAccount)
    )
  }

  func records() throws -> [BuzzPushEndpointGrantRecord] {
    var query = baseQuery(account: Self.recordsAccount)
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    if status == errSecItemNotFound { return [] }
    guard status == errSecSuccess, let data = result as? Data else {
      throw keychainError(status, operation: "read")
    }
    do {
      return try JSONDecoder().decode([BuzzPushEndpointGrantRecord].self, from: data)
    } catch {
      throw NSError(
        domain: "BuzzPushEndpointGrantStore",
        code: 1,
        userInfo: [NSLocalizedDescriptionKey: "Stored endpoint grants are invalid: \(error)"]
      )
    }
  }

  func save(_ record: BuzzPushEndpointGrantRecord) throws {
    var all = try records()
    all.removeAll {
      $0.gatewayOrigin == record.gatewayOrigin && $0.relayOrigin == record.relayOrigin
        && $0.appProfile == record.appProfile
    }
    all.append(record)
    try replace(all, account: Self.recordsAccount)
  }

  func removeRecord(gatewayOrigin: String, relayOrigin: String, appProfile: String) throws {
    var all = try records()
    all.removeAll {
      $0.gatewayOrigin == gatewayOrigin && $0.relayOrigin == relayOrigin
        && $0.appProfile == appProfile
    }
    try replace(all, account: Self.recordsAccount)
  }

  func removeRecords(gatewayOrigin: String, installationHandle: String) throws {
    var all = try records()
    all.removeAll {
      $0.gatewayOrigin == gatewayOrigin
        && $0.gatewayInstallationHandle == installationHandle
    }
    try replace(all, account: Self.recordsAccount)
  }

  func removeRecords(
    gatewayOrigin: String,
    installationHandle: String,
    relayPubkey: String
  ) throws {
    var all = try records()
    all.removeAll {
      $0.gatewayOrigin == gatewayOrigin
        && $0.gatewayInstallationHandle == installationHandle
        && $0.relayPubkey == relayPubkey
    }
    try replace(all, account: Self.recordsAccount)
  }

  func pendingEnrollment(
    gatewayOrigin: String,
    relayOrigin: String,
    appProfile: String
  ) throws -> BuzzPushPendingEnrollmentRecord? {
    try pendingEnrollments().first {
      $0.gatewayOrigin == gatewayOrigin && $0.relayOrigin == relayOrigin
        && $0.appProfile == appProfile
    }
  }

  func savePendingEnrollment(_ record: BuzzPushPendingEnrollmentRecord) throws {
    var all = try pendingEnrollments()
    all.removeAll {
      $0.gatewayOrigin == record.gatewayOrigin && $0.relayOrigin == record.relayOrigin
        && $0.appProfile == record.appProfile
    }
    all.append(record)
    try replace(all, account: Self.pendingAccount)
  }

  func removePendingEnrollment(
    gatewayOrigin: String,
    relayOrigin: String,
    appProfile: String
  ) throws {
    var all = try pendingEnrollments()
    all.removeAll {
      $0.gatewayOrigin == gatewayOrigin && $0.relayOrigin == relayOrigin
        && $0.appProfile == appProfile
    }
    try replace(all, account: Self.pendingAccount)
  }

  func gatewayCleanupStates() throws -> [BuzzPushGatewayCleanupState] {
    var query = baseQuery(account: Self.cleanupAccount)
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    if status == errSecItemNotFound { return [] }
    guard status == errSecSuccess, let data = result as? Data else {
      throw keychainError(status, operation: "read gateway cleanup")
    }
    return try JSONDecoder().decode([BuzzPushGatewayCleanupState].self, from: data)
  }

  func saveGatewayCleanupState(_ state: BuzzPushGatewayCleanupState) throws {
    var states = try gatewayCleanupStates()
    states.removeAll { $0.gatewayOrigin == state.gatewayOrigin }
    states.append(state)
    try replace(states, account: Self.cleanupAccount)
  }

  func removeGatewayCleanupState(gatewayOrigin: String) throws {
    var states = try gatewayCleanupStates()
    states.removeAll { $0.gatewayOrigin == gatewayOrigin }
    try replace(states, account: Self.cleanupAccount)
  }

  func replacementQueueState() throws -> BuzzPushReplacementQueueState {
    guard let data = try data(account: Self.replacementRelaysAccount) else {
      return BuzzPushReplacementQueueState(generation: 0, relayOrigins: [])
    }
    return try JSONDecoder().decode(BuzzPushReplacementQueueState.self, from: data)
  }

  func queueReplacementRelayOrigins(_ relayOrigins: [String]) throws {
    let current = try replacementQueueState()
    let (generation, overflow) = current.generation.addingReportingOverflow(1)
    guard !overflow else {
      throw NSError(
        domain: "BuzzPushEndpointGrantStore",
        code: 3,
        userInfo: [NSLocalizedDescriptionKey: "Replacement queue generation exhausted."]
      )
    }
    let state = BuzzPushReplacementQueueState(
      generation: generation,
      relayOrigins: Array(Set(current.relayOrigins + relayOrigins)).sorted()
    )
    try replaceValue(state, account: Self.replacementRelaysAccount)
  }

  func checkpointReplacementRelayOrigins(
    _ relayOrigins: [String],
    expectedGeneration: Int64
  ) throws -> Bool {
    var state = try replacementQueueState()
    guard state.generation == expectedGeneration else { return false }
    let completedOrigins = Set(relayOrigins)
    state.relayOrigins.removeAll { completedOrigins.contains($0) }
    try replaceValue(state, account: Self.replacementRelaysAccount)
    return true
  }

  func clearReplacementRelayOrigins() throws {
    let current = try replacementQueueState()
    let (generation, overflow) = current.generation.addingReportingOverflow(1)
    guard !overflow else {
      throw NSError(
        domain: "BuzzPushEndpointGrantStore",
        code: 3,
        userInfo: [NSLocalizedDescriptionKey: "Replacement queue generation exhausted."]
      )
    }
    try replaceValue(
      BuzzPushReplacementQueueState(generation: generation, relayOrigins: []),
      account: Self.replacementRelaysAccount
    )
  }

  private func pendingEnrollments() throws -> [BuzzPushPendingEnrollmentRecord] {
    var query = baseQuery(account: Self.pendingAccount)
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    if status == errSecItemNotFound { return [] }
    guard status == errSecSuccess, let data = result as? Data else {
      throw keychainError(status, operation: "read pending enrollment")
    }
    do {
      return try JSONDecoder().decode([BuzzPushPendingEnrollmentRecord].self, from: data)
    } catch {
      throw NSError(
        domain: "BuzzPushEndpointGrantStore",
        code: 2,
        userInfo: [NSLocalizedDescriptionKey: "Stored pending enrollments are invalid: \(error)"]
      )
    }
  }

  private func data(account: String) throws -> Data? {
    var query = baseQuery(account: account)
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    if status == errSecItemNotFound { return nil }
    guard status == errSecSuccess, let data = result as? Data else {
      throw keychainError(status, operation: "read legacy state")
    }
    return data
  }

  private func replace<T: Encodable>(_ values: [T], account: String) throws {
    try replaceValue(values, account: account)
  }

  private func delete(account: String) throws {
    let status = SecItemDelete(baseQuery(account: account) as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else {
      throw keychainError(status, operation: "delete legacy state")
    }
  }

  private func replaceValue<T: Encodable>(_ value: T, account: String) throws {
    let data = try JSONEncoder().encode(value)
    let updateStatus = SecItemUpdate(
      baseQuery(account: account) as CFDictionary,
      [kSecValueData as String: data] as CFDictionary
    )
    if updateStatus == errSecSuccess { return }
    guard updateStatus == errSecItemNotFound else {
      throw keychainError(updateStatus, operation: "update")
    }

    var add = baseQuery(account: account)
    add[kSecValueData as String] = data
    add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    let addStatus = SecItemAdd(add as CFDictionary, nil)
    guard addStatus == errSecSuccess else {
      throw keychainError(addStatus, operation: "add")
    }
  }

  private func baseQuery(account: String) -> [String: Any] {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: Self.service,
      kSecAttrAccount as String: account,
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
          "Endpoint grant Keychain \(operation) failed: \(SecCopyErrorMessageString(status, nil) ?? "unknown" as CFString)"
      ]
    )
  }
}

extension BuzzPushEndpointGrantRecord {
  var flutterArguments: [String: Any] {
    let arguments: [String: Any] = [
      "relayOrigin": relayOrigin,
      "relayPubkey": relayPubkey,
      "installationId": installationId,
      "endpointGrant": endpointGrant,
      "endpointHash": endpointHash,
      "appProfile": appProfile,
      "endpointEpoch": endpointEpoch,
      "generation": generation,
      "expiresAt": expiresAt,
    ]
    return arguments
  }
}
